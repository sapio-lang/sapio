// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Contract for managing movement of funds from cold to hot storage
use super::undo_send::UndoSendInternal;
use bitcoin::util::amount::CoinAmount;
use sapio::contract::*;
use sapio::*;
use sapio_base::timelocks::AnyRelTimeLock;

use schemars::*;
use serde::*;
use std::convert::{TryFrom, TryInto};
use std::rc::Rc;
use std::sync::Arc;

/// A Vault makes a "annuity chain" which pays out to `hot_storage` every `timeout` period for `n_steps`.
/// The funds in `hot_storage` are in an UndoSend contract for a timeout of
/// `mature`. At any time the remaining funds can be moved to `cold_storage`, which may vary based on the amount.
pub struct Vault {
    cold_storage: Rc<dyn Fn(CoinAmount, Context) -> Result<Compiled, CompilationError>>,
    hot_storage: bitcoin::Address,
    n_steps: u64,
    amount_step: CoinAmount,
    timeout: AnyRelTimeLock,
    mature: AnyRelTimeLock,
}

impl Vault {
    #[then]
    fn step(self, ctx: sapio::Context) {
        let mut ctx = ctx;
        let cold_storage_ctx = ctx.derive_str(Arc::new("cold".into()))?;
        let mut builder = ctx.template();
        builder = builder
            .add_output(
                self.amount_step.try_into()?,
                &UndoSendInternal {
                    from_contract: (self.cold_storage)(self.amount_step, cold_storage_ctx)?,
                    to_contract: Compiled::from_address(self.hot_storage.clone(), None),
                    timeout: self.mature,
                    amount: self.amount_step,
                },
                None,
            )?
            .set_sequence(0, self.timeout)?;

        if self.n_steps > 1 {
            let sub_amount = bitcoin::Amount::try_from(self.amount_step)
                .map_err(|_e| contract::CompilationError::TerminateCompilation)?
                .checked_mul(self.n_steps - 1)
                .ok_or(contract::CompilationError::TerminateCompilation)?;
            let sub_vault = Vault {
                cold_storage: self.cold_storage.clone(),
                hot_storage: self.hot_storage.clone(),
                n_steps: self.n_steps - 1,
                amount_step: self.amount_step,
                timeout: self.timeout,
                mature: self.mature,
            };
            builder.add_output(sub_amount, &sub_vault, None)?
        } else {
            builder
        }
        .into()
    }
    #[then]
    fn to_cold(self, ctx: sapio::Context) {
        let mut ctx = ctx;
        let amount = bitcoin::Amount::try_from(self.amount_step)
            .map_err(|_e| contract::CompilationError::TerminateCompilation)?
            .checked_mul(self.n_steps)
            .ok_or(contract::CompilationError::TerminateCompilation)?;
        let cold_storage_ctx = ctx.derive_str(Arc::new("cold".into()))?;
        ctx.template()
            .add_output(
                amount,
                &(self.cold_storage)(amount.into(), cold_storage_ctx)?,
                None,
            )?
            .into()
    }
}

impl Contract for Vault {
    declare! {then, Self::step, Self::to_cold}
    declare! {non updatable}
}

#[derive(JsonSchema, Deserialize)]
/// A specialization of `Vault` where cold storage is a regular `bitcoin::Address`
pub struct VaultAddress {
    /// # Address for Cold Storage
    cold_storage: bitcoin::Address,
    /// # Address for Hot Storage
    hot_storage: bitcoin::Address,
    /// # Number of Steps
    n_steps: u64,
    /// # Amount per Step
    amount_step: CoinAmount,
    /// # How long between steps
    timeout: AnyRelTimeLock,
    /// # How long before hot wallet spendable
    mature: AnyRelTimeLock,
}

impl From<VaultAddress> for Vault {
    fn from(v: VaultAddress) -> Self {
        Vault {
            cold_storage: Rc::new({
                let cs = v.cold_storage.clone();
                move |_a, _ctx| Ok(Compiled::from_address(cs.clone(), None))
            }),
            hot_storage: v.hot_storage,
            n_steps: v.n_steps,
            amount_step: v.amount_step,
            timeout: v.timeout,
            mature: v.mature,
        }
    }
}

#[derive(JsonSchema, Deserialize)]
/// # Value Split Tree Payment Vault
/// A specialization of `Vault` where cold storage is a tree payment to a `bitcoin::Address`
/// split up based on a max amount per address
pub struct VaultTree {
    /// # Cold Storage Target
    cold_storage: bitcoin::Address,
    /// # Max Funds per Cold Storage Addreess
    max_per_address: CoinAmount,
    /// # Radix for the split tree
    radix: usize,
    /// # A Hot Storage Address
    hot_storage: bitcoin::Address,
    /// # How many iterations of the contract to run
    n_steps: u64,
    /// # How much funds per step
    amount_step: CoinAmount,
    /// # How long between steps
    timeout: AnyRelTimeLock,
    /// # How long before hot wallet spendable
    mature: AnyRelTimeLock,
}

impl TryFrom<VaultTree> for Vault {
    type Error = CompilationError;
    fn try_from(v: VaultTree) -> Result<Self, CompilationError> {
        Ok(Vault {
            cold_storage: Rc::new({
                let cs = v.cold_storage.clone();
                let max: bitcoin::Amount = bitcoin::Amount::try_from(v.max_per_address)
                    .map_err(|_| CompilationError::TerminateCompilation)?;
                let rad = v.radix;
                move |a, ctx| {
                    let mut amt: bitcoin::Amount = bitcoin::Amount::try_from(a)
                        .map_err(|_| CompilationError::TerminateCompilation)?;
                    let mut pmts = vec![];
                    while amt > max {
                        pmts.push(super::treepay::Payment {
                            amount: max.into(),
                            address: cs.clone(),
                        });
                        amt -= max;
                    }
                    if amt > bitcoin::Amount::from_sat(0) {
                        pmts.push(super::treepay::Payment {
                            amount: max.into(),
                            address: cs.clone(),
                        });
                    }
                    ctx.compile(super::treepay::TreePay {
                        participants: pmts,
                        radix: rad,
                    })
                }
            }),
            hot_storage: v.hot_storage,
            n_steps: v.n_steps,
            amount_step: v.amount_step,
            timeout: v.timeout,
            mature: v.mature,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use sapio_base::effects::EffectPath;
    use sapio_base::plugin_args::CreateArgs;
    use sapio_ctv_emulator_trait::CTVAvailable;
    #[derive(JsonSchema, Deserialize)]
    enum Versions {
        ForAddress(VaultAddress),
        ForTree(VaultTree),
    }
    impl TryFrom<Versions> for Vault {
        type Error = CompilationError;
        fn try_from(v: Versions) -> Result<Vault, CompilationError> {
            match v {
                Versions::ForAddress(a) => Ok(a.into()),
                Versions::ForTree(t) => t.try_into(),
            }
        }
    }
    #[test]
    fn example() -> Result<(), Box<dyn std::error::Error>> {
        let string =  "{\"arguments\":{\"ForAddress\":{\"amount_step\":{\"Sats\":100},\"cold_storage\":\"bcrt1qumrrqgt7e3a7damzm8x97m6sjs20u8hjw2hcjj\",\"hot_storage\":\"bcrt1qumrrqgt7e3a7damzm8x97m6sjs20u8hjw2hcjj\",\"mature\":{\"RH\":10},\"n_steps\":10,\"timeout\":{\"RH\":5}}},\"context\":{\"amount\":1,\"network\":\"Regtest\"}}";
        let v: CreateArgs<Versions> = serde_json::from_str(string)?;
        let ctx = Context::new(
            v.context.network,
            v.context.amount,
            Arc::new(CTVAvailable),
            EffectPath::try_from("dlc").unwrap(),
            Arc::new(v.context.effects),
            None
        );
        Vault::try_from(v.arguments)?.compile(ctx)?;
        Ok(())
    }
}
