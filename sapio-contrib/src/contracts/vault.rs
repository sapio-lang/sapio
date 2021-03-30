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

/// A Vault makes a "annuity chain" which pays out to `hot_storage` every `timeout` period for `n_steps`.
/// The funds in `hot_storage` are in an UndoSend contract for a timeout of
/// `mature`. At any time the remaining funds can be moved to `cold_storage`, which may vary based on the amount.
pub struct Vault {
    cold_storage: Rc<dyn Fn(CoinAmount, &Context) -> Result<Compiled, CompilationError>>,
    hot_storage: bitcoin::Address,
    n_steps: u64,
    amount_step: CoinAmount,
    timeout: AnyRelTimeLock,
    mature: AnyRelTimeLock,
}

impl Vault {
    then! {fn step(self, ctx) {
        let builder = ctx.template()
        .add_output(self.amount_step.try_into()?,
                &UndoSendInternal {
                    from_contract: (self.cold_storage)(self.amount_step, ctx)?,
                    to_contract: Compiled::from_address(self.hot_storage.clone(), None),
                    timeout: self.mature,
                    amount: self.amount_step.into(),
                }, None)?
       .set_sequence(0, self.timeout)?;

        if self.n_steps > 1 {
            let sub_amount = bitcoin::Amount::try_from(self.amount_step).map_err(|_e| contract::CompilationError::TerminateCompilation)?.checked_mul(self.n_steps - 1).ok_or(contract::CompilationError::TerminateCompilation)?;
            let sub_vault = Vault {
                cold_storage: self.cold_storage.clone(),
                hot_storage: self.hot_storage.clone(),
                n_steps: self.n_steps -1,
                amount_step: self.amount_step,
                timeout: self.timeout,
                mature: self.mature,

            };
            builder.add_output(sub_amount, &sub_vault, None)?
        } else {
            builder
        }.into()
    }}
    then! {fn to_cold (self, ctx) {
        let amount = bitcoin::Amount::try_from(self.amount_step).map_err(|_e| contract::CompilationError::TerminateCompilation)?.checked_mul(self.n_steps).ok_or(contract::CompilationError::TerminateCompilation)?;
        ctx.template()
            .add_output(amount, &(self.cold_storage)(amount.into(), ctx)?, None)?
            .into()
    }}
}

impl Contract for Vault {
    declare! {then, Self::step, Self::to_cold}
    declare! {non updatable}
}

/// A specialization of `Vault` where cold storage is a regular `bitcoin::Address`
#[derive(JsonSchema, Deserialize)]
pub struct VaultAddress {
    cold_storage: bitcoin::Address,
    hot_storage: bitcoin::Address,
    n_steps: u64,
    amount_step: CoinAmount,
    timeout: AnyRelTimeLock,
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

/// A specialization of `Vault` where cold storage is a tree payment to a `bitcoin::Address`
/// split up based on a max amount per address
#[derive(JsonSchema, Deserialize)]
pub struct VaultTree {
    cold_storage: bitcoin::Address,
    max_per_address: CoinAmount,
    radix: usize,
    hot_storage: bitcoin::Address,
    n_steps: u64,
    amount_step: CoinAmount,
    timeout: AnyRelTimeLock,
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
