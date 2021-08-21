// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use batching_trait::{BatchingTraitVersion0_1_1, Payment};
#[deny(missing_docs)]
use sapio::contract::*;
use sapio::util::amountrange::*;
use sapio::*;
use sapio_base::timelocks::{AnyRelTimeLock};
use sapio_wasm_plugin::client::*;
use sapio_wasm_plugin::*;
use schemars::*;
use serde::*;
use std::collections::VecDeque;

// Documentation placed here will be visible to users!
/// # Tree Payment Contract
/// This contract is used to help decongest bitcoin while giving users full
/// confirmation of transfer.
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct TreePay {
    /// # Payments
    /// all of the payments needing to be sent
    pub participants: Vec<Payment>,
    /// # Tree Branching Factor
    /// the radix of the tree to build. Optimal for users should be around 4 or
    /// 5 (with CTV, not emulators).
    pub radix: usize,
    #[serde(with = "bitcoin::util::amount::serde::as_sat")]
    #[schemars(with = "u64")]
    /// # Fee Sats (per tx)
    /// The amount of fees per transaction to allocate.
    pub fee_sats_per_tx: bitcoin::util::amount::Amount,
    /// # Relative Timelock Backpressure
    /// When enabled, exert backpressure by slowing down tree expansion node by
    /// node either by time or blocks
    pub timelock_backpressure: Option<AnyRelTimeLock>,
}

use bitcoin::util::amount::Amount;
struct PayThese {
    contracts: Vec<(Amount, Box<dyn Compilable>)>,
    fees: Amount,
    delay: Option<AnyRelTimeLock>,
}
impl PayThese {
    then! {
        fn expand(self, ctx) {
            let mut bld = ctx.template();
            for (amt, ct) in self.contracts.iter() {
                bld = bld.add_output(*amt, ct.as_ref(), None)?;
            }
            if let Some(delay) = self.delay {
                bld = bld.set_sequence(0, delay)?;
            }
            bld.add_fees(self.fees)?.into()
        }
    }

    fn total_to_pay(&self) -> Amount {
        let mut amt = self.fees;
        for (x, _) in self.contracts.iter() {
            amt += *x;
        }
        amt
    }
}
impl Contract for PayThese {
    declare! {then, Self::expand}
    declare! {non updatable}
}
impl TreePay {
    then! {
        fn expand(self, ctx) {
            let mut queue: VecDeque<(Amount, Box<dyn Compilable>)> = self
                .participants
                .iter()
                .map(|payment| {
                    let mut amt = AmountRange::new();
                    amt.update_range(payment.amount);
                    let b: Box<dyn Compilable> =
                        Box::new(Compiled::from_address(payment.address.clone(), Some(amt)));
                    (payment.amount, b)
                })
                .collect();

            loop {
                let v: Vec<_> = queue
                    .drain(0..std::cmp::min(self.radix, queue.len()))
                    .collect();
                if queue.len() == 0 {
                    let mut builder = ctx.template();
                    for pay in v.iter() {
                        builder = builder.add_output(pay.0, pay.1.as_ref(), None)?;
                        if let Some(timelock) = self.timelock_backpressure {
                            builder = builder.set_sequence(0, timelock)?;
                        }
                    }
                    builder = builder.add_fees(self.fee_sats_per_tx)?;
                    return builder.into();
                } else {
                    let pay = Box::new(PayThese {
                        contracts: v,
                        fees: self.fee_sats_per_tx,
                        delay: self.timelock_backpressure,
                    });
                    queue.push_back((pay.total_to_pay(), pay))
                }
            }
        }
    }
}
impl Contract for TreePay {
    declare! {then, Self::expand}
    declare! {non updatable}
}

/// # Different Calling Conventions to create a Treepay
#[derive(Serialize, Deserialize, JsonSchema)]
enum Versions {
    /// # Standard Tree Pay
    TreePay(TreePay),
    /// # Advanced Tree Pay
    Advanced {
        /// # Standard Tree Pay
        main_arguments: TreePay,
        /// # A Random Field For Example
        random_field: u8,
    },
    /// # Batching Trait API
    BatchingTraitVersion0_1_1(BatchingTraitVersion0_1_1),
}
impl From<BatchingTraitVersion0_1_1> for TreePay {
    fn from(args: BatchingTraitVersion0_1_1) -> Self {
        TreePay {
            participants: args.payments,
            radix: 4,
            // estimate fees to be 4 outputs and 1 input + change
            fee_sats_per_tx: args.feerate_per_byte * ((4 * 41) + 41 + 10),
            timelock_backpressure: None,
        }
    }
}
impl From<Versions> for TreePay {
    fn from(v: Versions) -> TreePay {
        match v {
            Versions::TreePay(v) => v,
            Versions::Advanced { main_arguments, .. } => main_arguments,
            Versions::BatchingTraitVersion0_1_1(v) => v.into(),
        }
    }
}
REGISTER![[TreePay, Versions], "logo.png"];
