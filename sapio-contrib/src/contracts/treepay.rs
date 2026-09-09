// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! contracts for paying a large set of recipients fee efficiently
use sapio::contract::*;
use sapio::*;

use schemars::*;
use serde::*;
use std::convert::TryInto;

/// instructions to send an amount of coin to an address
#[derive(JsonSchema, Serialize, Deserialize, Clone)]
pub struct Payment {
    /// The amount of coin to send
    pub amount: bitcoin::util::amount::CoinAmount,
    /// # Address
    /// The Address to send to
    pub address: bitcoin::Address,
}
/// Create a tree of payments with a given radix
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct TreePay {
    /// the list of payments to create
    pub participants: Vec<Payment>,
    /// the radix to use (4 or 5 near optimal, depending on if CTV emulation is used this may be inaccurate)
    pub radix: usize,
}

impl TreePay {
    #[then]
    fn expand(self, ctx: sapio::Context) {
        let mut builder = ctx.template();
        if self.participants.len() > self.radix {
            let chunk_size = self.participants.len().div_ceil(self.radix);
            for c in self.participants.chunks(chunk_size) {
                let mut amt = bitcoin::util::amount::Amount::from_sat(0);
                for Payment { amount, .. } in c {
                    amt = amt
                        .checked_add((*amount).try_into()?)
                        .ok_or(CompilationError::OutOfFunds)?;
                }
                builder = builder.add_output(
                    amt,
                    &TreePay {
                        participants: c.to_vec(),
                        radix: self.radix,
                    },
                    None,
                )?;
            }
        } else {
            for Payment { amount, address } in self.participants.iter() {
                builder = builder.add_output(
                    (*amount).try_into()?,
                    &Compiled::from_address(address.clone(), None),
                    None,
                )?;
            }
        }
        builder.into()
    }
}

impl Contract for TreePay {
    declare! {then, Self::expand}
    declare! {non updatable}

    fn ensure_amount(&self, _ctx: Context) -> Result<bitcoin::Amount, CompilationError> {
        if self.radix < 2 || self.participants.is_empty() {
            return Err(CompilationError::Custom(
                "TreePay needs recipients and radix >= 2".into(),
            ));
        }
        self.participants
            .iter()
            .try_fold(bitcoin::Amount::ZERO, |total, payment| {
                let amount: bitcoin::Amount = payment.amount.try_into()?;
                if amount == bitcoin::Amount::ZERO {
                    return Err(CompilationError::Custom(
                        "TreePay payments must be positive".into(),
                    ));
                }
                total
                    .checked_add(amount)
                    .ok_or(CompilationError::OutOfFunds)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{address, context};

    fn payments(count: usize) -> Vec<Payment> {
        (0..count)
            .map(|_| Payment {
                amount: bitcoin::Amount::from_sat(1000).into(),
                address: address(1),
            })
            .collect()
    }

    #[test]
    fn uneven_tree_preserves_payments_and_radix() {
        fn leaves(object: &Compiled, radix: usize) -> (usize, u64) {
            let Some(template) = object.ctv_to_tx.values().next() else {
                return (1, 0);
            };
            assert!(template.outputs.len() <= radix);
            template
                .outputs
                .iter()
                .fold((0, 0), |(count, total), output| {
                    if output.contract.ctv_to_tx.is_empty() {
                        (count + 1, total + output.amount.as_sat())
                    } else {
                        let (n, sats) = leaves(&output.contract, radix);
                        (count + n, total + sats)
                    }
                })
        }
        let object = TreePay {
            participants: payments(11),
            radix: 3,
        }
        .compile(context(11000))
        .unwrap();
        object.validate().unwrap();
        assert_eq!(leaves(&object, 3), (11, 11000));
    }

    #[test]
    fn invalid_radices_empty_and_underfunded_trees_fail() {
        for radix in [0, 1] {
            assert!(TreePay {
                participants: payments(2),
                radix
            }
            .compile(context(2000))
            .is_err());
        }
        assert!(TreePay {
            participants: vec![],
            radix: 2
        }
        .compile(context(0))
        .is_err());
        let mut zero_payment = payments(1);
        zero_payment[0].amount = bitcoin::Amount::ZERO.into();
        assert!(TreePay {
            participants: zero_payment,
            radix: 2
        }
        .compile(context(0))
        .is_err());
        assert!(TreePay {
            participants: payments(2),
            radix: 2
        }
        .compile(context(1999))
        .is_err());
    }
}
