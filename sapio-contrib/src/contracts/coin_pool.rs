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
use sapio_base::timelocks::AnyRelTimeLock;
use sapio_base::Clause;

use sapio_data_repr::HasSapioModuleSchema;
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
                .add_output(
                    Amount::from_sat(a.refunds.iter().map(|x| Amount::from(x.1).as_sat()).sum()),
                    &a,
                    None,
                )?
                .add_output(
                    Amount::from_sat(b.refunds.iter().map(|x| Amount::from(x.1).as_sat()).sum()),
                    &b,
                    None,
                )?
                .into()
        } else {
            let mut builder = ctx.template();
            for (cmp, amt) in self.refunds.iter() {
                builder = builder.add_output((*amt).into(), &*cmp.lock().unwrap(), None)?;
            }
            builder.into()
        }
    }
    #[guard]
    /// everyone has signed off on the transaction
    fn all_approve(self, _ctx: Context) {
        Clause::Threshold(self.clauses.len(), self.clauses.clone())
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
            let mut tmpl = ctx.template().add_amount(coin_pool.external_amount.into());
            for (to, amt) in coin_pool.payouts.iter() {
                tmpl = tmpl.add_output((*amt).into(), &*to.lock().unwrap(), None)?;
            }
            for seq in coin_pool.add_inputs.iter() {
                tmpl = tmpl.add_sequence().set_sequence(-1, *seq)?;
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
#[derive(Deserialize)]
pub enum UpdateTypes {
    /// # Normal Update
    Basic {
        /// the contracts to pay into
        #[serde(skip_serializing_if = "Option::is_none", default)]
        // TODO: Taproot fix encoding
        // #[schemars(with = "Option<Vec<(bitcoin::hashes::sha256::Hash, AmountF64)>>")]
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
impl HasSapioModuleSchema for UpdateTypes {
    fn get_schema() -> sapio_data_repr::SapioModuleSchema {
        todo!()
    }
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
}
