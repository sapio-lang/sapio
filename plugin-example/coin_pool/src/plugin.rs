// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
#[deny(missing_docs)]
use crate::sapio_base::Clause;
use bitcoin::util::amount::Amount;
use bitcoin::util::amount::CoinAmount;
use sapio::contract::*;
use sapio::*;
use sapio_contrib::contracts::coin_pool::CoinPool;
use sapio_wasm_plugin::client::*;
use sapio_wasm_plugin::*;
use schemars::*;
use serde::*;
use std::convert::{TryFrom, TryInto};
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
enum PoolTypes {
    /// # Expert Mode
    /// This allows you to specify sub plugins to call out to for every participant
    PluginPool {
        clauses: Vec<Clause>,
        refunds: Vec<Payout>,
    },
    /// # Basic Mode
    ///
    /// Accepts a list of amounts and keys and derives all relevant state.
    Basic(
        /// # Add Multiple Payments
        #[schemars(length(min = 1))]
        Vec<(CoinAmount, bitcoin::PublicKey)>,
    ),
}

impl TryFrom<PoolTypes> for CoinPool {
    type Error = CompilationError;
    fn try_from(v: PoolTypes) -> Result<CoinPool, CompilationError> {
        match v {
            PoolTypes::Basic(payouts) => {
                let refunds: Vec<(Arc<Mutex<dyn Compilable>>, Amount)> = payouts
                    .iter()
                    .map(|(a, k)| {
                        let compilable: Arc<Mutex<dyn Compilable>> =
                            Arc::new(Mutex::new(k.clone()));
                        Ok((compilable, Amount::try_from(*a)?))
                    })
                    .collect::<Result<Vec<_>, CompilationError>>()?;
                Ok(CoinPool {
                    clauses: payouts
                        .iter()
                        .map(|(_, k)| Clause::Key(k.clone()))
                        .collect(),
                    refunds,
                })
            }
            PoolTypes::PluginPool { clauses, refunds } => {
                let mut processed_refunds = vec![];
                for payout in refunds.iter() {
                    if let Some(key) = payout.payout_handle.to_key() {
                        if let Some(compiled) = create_contract_by_key(
                            &key,
                            serde_json::from_str(&payout.payout_args)
                                .map_err(|_| CompilationError::TerminateCompilation)?,
                            payout.amount,
                        ) {
                            let compilable: Arc<Mutex<dyn Compilable>> =
                                Arc::new(Mutex::new(compiled));
                            processed_refunds.push((compilable, payout.amount));
                            continue;
                        }
                    }
                    return Err(CompilationError::TerminateCompilation);
                }
                Ok(CoinPool {
                    clauses: clauses,
                    refunds: processed_refunds,
                })
            }
        }
    }
}
REGISTER![[CoinPool, PoolTypes], "logo.png"];
