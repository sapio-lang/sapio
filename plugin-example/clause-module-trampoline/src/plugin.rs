// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
//! Clause Module Example

#![deny(missing_docs)]
use sapio_wasm_plugin::plugin_handle::PluginHandle;
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
use serde_json::Value;
use sapio_trait::SapioJSONTrait;
use serde::Serialize;
use bitcoin::XOnlyPublicKey;
use std::str::FromStr;

/// Same Inner type as the wrapped module
#[derive(JsonSchema, Deserialize, Serialize, Clone)]
pub struct GetClause {
    // TODO: Taproot Fix Encoding
    #[schemars(with = "bitcoin::hashes::sha256::Hash")]
    alice: bitcoin::XOnlyPublicKey,
    // TODO: Taproot Fix Encoding
    #[schemars(with = "bitcoin::hashes::sha256::Hash")]
    bob: bitcoin::XOnlyPublicKey,
}

/// Wrapper to find the ClauseModule remotely
#[derive(JsonSchema, Deserialize)]
pub struct Wrapper {
    g: GetClause,
    v: ClauseModule<GetClause>
}



impl SapioJSONTrait for GetClause {
    fn get_example_for_api_checking() -> Value {
        serde_json::to_value(GetClause{
            alice: XOnlyPublicKey::from_str("01ba4719c80b6fe911b091a7c05124b64eeece964e09c058ef8f9805daca546b").unwrap(),
            bob: XOnlyPublicKey::from_str("01ba4719c80b6fe911b091a7c05124b64eeece964e09c058ef8f9805daca546c").unwrap()
        })
        .unwrap()
    }
}

impl Callable for Wrapper {
    type Output = Clause;
    fn call(&self, ctx: Context) -> Result<Clause, CompilationError> {
        let create_args: CreateArgs<GetClause> = CreateArgs {
            context: ContextualArguments {
                amount: ctx.funds(),
                network: ctx.network,
                effects: unsafe { ctx.get_effects_internal() }.as_ref().clone(),
                ordinals_info: ctx.get_ordinals().clone()
            },
            arguments: self.g.clone(),
        };
        self.v.clone().call(ctx.path(), &create_args)
    }
}

REGISTER![Wrapper, "logo.png"];
