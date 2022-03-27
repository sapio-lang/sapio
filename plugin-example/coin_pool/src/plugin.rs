// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
#[deny(missing_docs)]
use crate::sapio_base::Clause;

use sapio::contract::*;
use sapio::util::amountrange::AmountF64;
use sapio::*;
use sapio_contrib::contracts::coin_pool::CoinPool;
use sapio_wasm_plugin::client::*;
use sapio_wasm_plugin::*;
use schemars::*;
use serde::*;
use std::convert::TryFrom;
use std::sync::{Arc, Mutex};

/// # Payout Instructions
#[derive(JsonSchema, Deserialize)]
struct Payout {
    /// # Amount to Pay (BTC)
    amount: AmountF64,
    /// # Payout Plugin ID
    payout_handle: LookupFrom,
    /// # Arguments (as JSON) for Plugin
    payout_args: CreateArgs<String>,
}

/// # Plugin Based Payment Pool
/// A payment pool where there are a set of governing clauses and a set of
/// plugins based payouts.
#[derive(JsonSchema, Deserialize)]
enum PoolTypes {
    // TODO: 
    // Plugin serialization time should, unfortunately, not make calls to sub-plugins -- yet.
    // TO fix this will require figuring out how to pass a Context object into the TryFrom

    // /// # Expert Mode
    // /// This allows you to specify sub plugins to call out to for every participant
    // PluginPool {
    //     clauses: Vec<Clause>,
    //     refunds: Vec<Payout>,
    // },
    /// # Basic Mode
    ///
    /// Accepts a list of amounts and keys and derives all relevant state.
    Basic(
        /// # Add Multiple Payments
        #[schemars(length(min = 1))]
        Vec<SimplePayment>,
    ),
}

/// # Payment to Key
#[derive(JsonSchema, Deserialize)]
pub struct SimplePayment {
    /// # The Key that Votes & Redeems Funds
    // TODO: Taproot Fix Encoding
    #[schemars(with = "bitcoin::hashes::sha256::Hash")]
    key: bitcoin::XOnlyPublicKey,
    /// # Amount to Pay in BTC
    amount: AmountF64,
}

impl TryFrom<PoolTypes> for CoinPool {
    type Error = CompilationError;
    fn try_from(v: PoolTypes) -> Result<CoinPool, CompilationError> {
        match v {
            PoolTypes::Basic(payouts) => {
                let refunds: Vec<(Arc<Mutex<dyn Compilable>>, AmountF64)> = payouts
                    .iter()
                    .map(|s| {
                        let compilable: Arc<Mutex<dyn Compilable>> =
                            Arc::new(Mutex::new(s.key.clone()));
                        Ok((compilable, s.amount))
                    })
                    .collect::<Result<Vec<_>, CompilationError>>()?;
                Ok(CoinPool {
                    clauses: payouts.iter().map(|s| Clause::Key(s.key.clone())).collect(),
                    refunds,
                })
            }
            //PoolTypes::PluginPool { clauses, refunds } => {
            //    let mut processed_refunds = vec![];
            //    for payout in refunds.iter() {
            //        let key = payout
            //            .payout_handle
            //            .to_key()
            //            .ok_or(CompilationError::TerminateCompilation)?;
            //        let plugin_ctx = ctx.derive_str(Arc::new("pool_plugin".into()))?,
            //        let compiled = create_contract_by_key(plugin_ctx, &key, payout.payout_args.clone())?;
            //        let compilable: Arc<Mutex<dyn Compilable>> = Arc::new(Mutex::new(compiled));
            //        processed_refunds.push((compilable, payout.amount));
            //    }
            //    Ok(CoinPool {
            //        clauses: clauses,
            //        refunds: processed_refunds,
            //    })
            //}
        }
    }
}
REGISTER![[CoinPool, PoolTypes], "logo.png"];
