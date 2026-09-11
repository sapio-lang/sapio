// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Clause Module for showing non-sapio compiled object types

#![deny(missing_docs)]
use sapio::contract::*;
use sapio::*;
use sapio_base::Clause;
use sapio_wasm_plugin::client::plugin::Callable;
#[cfg(target_arch = "wasm32")]
use sapio_wasm_plugin::{optional_logo, REGISTER};
use schemars::JsonSchema;
use serde::Deserialize;

/// Get a Clause for two parties to sign together
#[derive(JsonSchema, Deserialize)]
pub struct GetClause {
    // TODO: Taproot Fix Encoding
    alice: bitcoin::XOnlyPublicKey,
    // TODO: Taproot Fix Encoding
    bob: bitcoin::XOnlyPublicKey,
}

impl Callable for GetClause {
    type Output = Clause;
    fn call(&self, _ctx: Context) -> Result<Clause, CompilationError> {
        Ok(Clause::And(vec![
            Clause::Key(self.alice).into(),
            Clause::Key(self.bob).into(),
        ]))
    }
}

#[cfg(target_arch = "wasm32")]
REGISTER![GetClause, "logo.png"];
