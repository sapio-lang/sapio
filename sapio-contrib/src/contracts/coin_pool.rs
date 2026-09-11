// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! coin_pool has a contract `CoinPool` for sharing a UTXO
use bitcoin::Amount;
use sapio::contract::*;
use sapio::util::amountrange::AmountF64;
use sapio::*;
use sapio_base::miniscript::{Threshold, ThresholdError};
use sapio_base::timelocks::AnyRelTimeLock;
use sapio_base::Clause;

use schemars::JsonSchema;
use serde::Deserialize;
use std::convert::{TryFrom, TryInto};
use std::sync::{Arc, Mutex};
type Payouts = Vec<(Arc<Mutex<dyn Compilable>>, AmountF64)>;
/// A CoinPool is a contract that allows a group of individuals to
/// cooperatively share a UTXO.
pub struct CoinPool {
    /// The list of stakeholders
    pub clauses: Vec<Clause>,
    /// How to refund people if no update agreed on
    pub refunds: Payouts,
}
/// Helper
fn default_coerce(
    k: <CoinPool as Contract>::StatefulArguments,
) -> Result<UpdateTypes, CompilationError> {
    Ok(k)
}

impl CoinPool {
    fn refund_total(refunds: &Payouts) -> Result<Amount, CompilationError> {
        refunds.iter().try_fold(Amount::ZERO, |total, (_, amount)| {
            total
                .checked_add((*amount).into())
                .ok_or(CompilationError::OutOfFunds)
        })
    }
    /// cuts the pool in half in order to remove an offline or malicious participant
    #[then]
    fn bisect_offline(self, ctx: sapio::Context) {
        if self.clauses.len() >= 2 {
            let l = self.clauses.len();
            let a = CoinPool {
                clauses: self.clauses[0..l / 2].into(),
                refunds: self.refunds[0..l / 2].into(),
            };

            let b = CoinPool {
                clauses: self.clauses[l / 2..].into(),
                refunds: self.refunds[l / 2..].into(),
            };

            ctx.template()
                .add_output(Self::refund_total(&a.refunds)?, &a, None)?
                .add_output(Self::refund_total(&b.refunds)?, &b, None)?
                .into()
        } else {
            let mut builder = ctx.template();
            for (cmp, amt) in self.refunds.iter() {
                builder = builder.add_output((*amt).into(), &*cmp.lock().unwrap(), None)?;
            }
            builder.into()
        }
    }
    #[guard(policy)]
    /// everyone has signed off on the transaction
    fn all_approve(self, _ctx: Context) -> Result<Clause, ThresholdError> {
        Ok(Clause::Thresh(Threshold::new(
            self.clauses.len(),
            self.clauses.clone().into_iter().map(Into::into).collect(),
        )?))
    }
    /// move the coins to the next state -- payouts may recursively contain pools itself
    #[continuation(
        web_api,
        guarded_by = "[Self::all_approve]",
        coerce_args = "default_coerce"
    )]
    fn next_pool(self, ctx: sapio::Context, o: UpdateTypes) {
        let o2: Option<CoinPoolUpdate> = o.try_into()?;
        if let Some(coin_pool) = o2 {
            let external: Amount = coin_pool.external_amount.into();
            ctx.funds()
                .checked_add(external)
                .ok_or(CompilationError::OutOfFunds)?;
            if external != Amount::ZERO && coin_pool.add_inputs.is_empty() {
                return Err(CompilationError::Custom(
                    "External funds require an additional input".into(),
                ));
            }
            let mut tmpl = ctx.template();
            for seq in coin_pool.add_inputs.iter() {
                tmpl = tmpl.add_sequence().set_sequence(-1, *seq)?;
            }
            tmpl = tmpl.add_amount(coin_pool.external_amount.into())?;
            for (to, amt) in coin_pool.payouts.iter() {
                tmpl = tmpl.add_output((*amt).into(), &*to.lock().unwrap(), None)?;
            }
            tmpl.into()
        } else {
            empty()
        }
    }
}

/// `CoinPoolUpdate` allows updating a `CoinPool` to a new state.
pub struct CoinPoolUpdate {
    /// the contracts to pay into
    payouts: Payouts,
    /// if we should add any inputs to the transaction, and if so, what the
    /// sequences should be set to.
    add_inputs: Vec<AnyRelTimeLock>,
    /// If the external inputs are contributing funds -- this allows two
    /// coinpools to merge.
    /// TODO: Allow different indexes?
    external_amount: AmountF64,
}

/// `CoinPoolUpdate` allows updating a `CoinPool` to a new state.
#[derive(Deserialize, JsonSchema)]
pub enum UpdateTypes {
    /// # Normal Update
    Basic {
        /// the contracts to pay into
        #[serde(skip_serializing_if = "Option::is_none", default)]
        #[schemars(with = "Option<Vec<(String, AmountF64)>>")]
        payouts: Option<Vec<(bitcoin::XOnlyPublicKey, AmountF64)>>,
        /// If the external inputs are contributing funds -- this allows two
        /// coinpools to merge.
        /// TODO: Allow different indexes?
        external_amount: AmountF64,
        /// if we should add any inputs to the transaction, and if so, what the
        /// sequences should be set to.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        add_inputs: Option<Vec<AnyRelTimeLock>>,
    },
    /// # Update without Args
    NoUpdate,
}
impl Default for UpdateTypes {
    fn default() -> Self {
        UpdateTypes::NoUpdate
    }
}
impl StatefulArgumentsTrait for UpdateTypes {}
impl TryFrom<UpdateTypes> for Option<CoinPoolUpdate> {
    type Error = CompilationError;
    fn try_from(u: UpdateTypes) -> Result<Option<CoinPoolUpdate>, CompilationError> {
        match u {
            UpdateTypes::Basic {
                add_inputs,
                external_amount,
                payouts,
            } => Ok(Some(CoinPoolUpdate {
                add_inputs: add_inputs.unwrap_or_default(),
                external_amount,
                payouts: payouts
                    .unwrap_or_default()
                    .iter()
                    .map(|(a, b)| {
                        let k: Arc<Mutex<dyn Compilable>> = Arc::new(Mutex::new(*a));
                        (k, (*b))
                    })
                    .collect(),
            })),
            _ => Ok(None),
        }
    }
}

impl Contract for CoinPool {
    declare! {then, Self::bisect_offline}
    declare! {updatable<UpdateTypes>, Self::next_pool}

    fn ensure_amount(&self, _ctx: Context) -> Result<Amount, CompilationError> {
        if self.clauses.is_empty() || self.clauses.len() != self.refunds.len() {
            return Err(CompilationError::Custom(
                "CoinPool needs one refund per participant".into(),
            ));
        }
        Self::refund_total(&self.refunds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{context, key};

    fn pool(n: u8) -> CoinPool {
        CoinPool {
            clauses: (1..=n).map(|n| Clause::Key(key(n))).collect(),
            refunds: (1..=n)
                .map(|n| {
                    (
                        Arc::new(Mutex::new(key(n))) as Arc<Mutex<dyn Compilable>>,
                        Amount::from_sat(1000).into(),
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn odd_pool_bisects_without_losing_funds() {
        let object = pool(3).compile(context(3000)).unwrap();
        object.validate().unwrap();
        let split = object.ctv_to_tx.values().next().unwrap();
        assert_eq!(
            split
                .outputs
                .iter()
                .map(|o| o.amount.to_sat())
                .collect::<Vec<_>>(),
            vec![1000, 2000]
        );
        assert_eq!(
            pool(3).guard_all_approve(context(3000)).unwrap(),
            Clause::Thresh(
                Threshold::new(
                    3,
                    (1..=3)
                        .map(|n| Clause::Key(key(n)))
                        .map(Into::into)
                        .collect()
                )
                .expect("valid threshold")
            )
        );
    }

    #[test]
    fn malformed_pool_and_unbacked_external_funds_fail() {
        let mut malformed = pool(2);
        malformed.refunds.pop();
        assert!(malformed.compile(context(2000)).is_err());
        assert!(pool(0).compile(context(0)).is_err());
        let mut overflow = pool(2);
        for (_, amount) in &mut overflow.refunds {
            *amount = Amount::from_sat(u64::MAX).into();
        }
        assert!(overflow.compile(context(u64::MAX)).is_err());
        assert!(pool(1)
            .continue_next_pool(
                context(1000),
                UpdateTypes::Basic {
                    payouts: None,
                    external_amount: Amount::from_sat(1).into(),
                    add_inputs: None
                }
            )
            .is_err());
    }
}
