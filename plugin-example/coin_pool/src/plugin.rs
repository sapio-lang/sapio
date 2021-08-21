// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.


#[deny(missing_docs)]
use sapio::contract::*;

use sapio::*;

use sapio_wasm_plugin::client::*;
use sapio_wasm_plugin::*;
use schemars::*;
use serde::*;

use std::convert::{TryFrom};

use crate::sapio_base::Clause;
use sapio_contrib::contracts::coin_pool::CoinPool;

use std::sync::{Arc, Mutex};

/// # Payout Instructions
#[derive(JsonSchema, Deserialize)]
struct Payout {
    /// # Amount to Pay (BTC)
    #[serde(with = "bitcoin::util::amount::serde::as_btc")]
    #[schemars(with = "f64")]
    amount: bitcoin::Amount,
    /// # Payout Plugin ID
    payout_handle: LookupFrom,
    /// # Arguments (as JSON) for Plugin
    payout_args: String,
}

/// # Plugin Based Payment Pool
/// A payment pool where there are a set of governing clauses and a set of
/// plugins based payouts.
#[derive(JsonSchema, Deserialize)]
struct PluginPool {
    clauses: Vec<Clause>,
    refunds: Vec<Payout>,
}

impl TryFrom<PluginPool> for CoinPool {
    type Error = CompilationError;
    fn try_from(v: PluginPool) -> Result<CoinPool, CompilationError> {
        let mut refunds = vec![];
        for payout in v.refunds.iter() {
            if let Some(key) = payout.payout_handle.to_key() {
                if let Some(compiled) = create_contract_by_key(
                    &key,
                    serde_json::from_str(&payout.payout_args)
                        .map_err(|_| CompilationError::TerminateCompilation)?,
                    payout.amount,
                ) {
                    let compilable: Arc<Mutex<dyn Compilable>> = Arc::new(Mutex::new(compiled));
                    refunds.push((compilable, payout.amount));
                    continue;
                }
            }
            return Err(CompilationError::TerminateCompilation);
        }
        Ok(CoinPool {
            clauses: v.clauses,
            refunds,
        })
    }
}
REGISTER![[CoinPool, PluginPool], "logo.png"];
