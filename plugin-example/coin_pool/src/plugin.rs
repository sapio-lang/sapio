// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Coin Pool

#![deny(missing_docs)]
use crate::sapio_base::Clause;
use sapio::contract::*;
use sapio::util::amountrange::AmountF64;
use sapio::*;
use sapio_contrib::contracts::coin_pool::CoinPool;
#[cfg(target_arch = "wasm32")]
use sapio_wasm_plugin::{optional_logo, REGISTER};
use schemars::*;
use serde::*;
use std::convert::TryFrom;
use std::sync::{Arc, Mutex};

/// # Plugin Based Payment Pool
/// A payment pool where there are a set of governing clauses and a set of
/// plugins based payouts.
#[derive(JsonSchema, Deserialize)]
enum PoolTypes {
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
        }
    }
}
#[cfg(target_arch = "wasm32")]
REGISTER![[CoinPool, PoolTypes], "logo.png"];
