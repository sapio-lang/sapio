// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[deny(missing_docs)]
use sapio_wasm_plugin::client::*;
use sapio_wasm_plugin::*;
use bitcoin::util::amount::CoinAmount;
use sapio::contract::*;
use sapio::*;
use sapio_base::timelocks::RelTime;
use sapio_base::Clause;
use schemars::JsonSchema;
use serde::Deserialize;
use std::convert::{TryFrom, TryInto};
use sapio_wasm_plugin::client::plugin::Callable;

#[derive(JsonSchema, Deserialize)]
pub struct GetClause {
    // TODO: Taproot Fix Encoding
    #[schemars(with = "bitcoin::hashes::sha256::Hash")]
    alice: bitcoin::XOnlyPublicKey,
    // TODO: Taproot Fix Encoding
    #[schemars(with = "bitcoin::hashes::sha256::Hash")]
    bob: bitcoin::XOnlyPublicKey,
}


impl Callable for GetClause {
    type Output = Clause;
    fn call(&self, ctx: Context) -> Result<Clause, CompilationError> {
        Ok(
        Clause::And(vec![Clause::Key(self.alice), Clause::Key(self.bob)])
        )
    }
}

REGISTER![GetClause, "logo.png"];
