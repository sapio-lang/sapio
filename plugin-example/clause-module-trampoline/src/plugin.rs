// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
//! Clause Module Example

#![deny(missing_docs)]
use sapio::contract::*;
use sapio::*;
use sapio_base::Clause;
use sapio_wasm_plugin::client::plugin::Callable;
use sapio_wasm_plugin::client::*;
use sapio_wasm_plugin::plugin_handle::PluginHandle;
use sapio_wasm_plugin::*;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// Same Inner type as the wrapped module
#[derive(JsonSchema, Deserialize, Serialize, Clone)]
pub struct GetClause {
    /// Alice's x-only public key.
    #[schemars(title = "Alice", schema_with = "sapio_base::schema::x_only_public_key")]
    alice: bitcoin::XOnlyPublicKey,
    /// Bob's x-only public key.
    #[schemars(title = "Bob", schema_with = "sapio_base::schema::x_only_public_key")]
    bob: bitcoin::XOnlyPublicKey,
}

/// Wrapper to find the ClauseModule remotely
#[derive(JsonSchema, Deserialize)]
pub struct Wrapper {
    /// Public keys supplied to the selected authorization implementation.
    #[schemars(title = "Parties")]
    g: GetClause,
    /// The caller supplies Parties as this module's arguments.
    #[schemars(title = "Authorization implementation")]
    v: ClauseModule<GetClause>,
}

impl Callable for Wrapper {
    type Output = Clause;
    fn call(&self, ctx: Context) -> Result<Clause, CompilationError> {
        let create_args: CreateArgs<GetClause> = CreateArgs {
            context: ContextualArguments {
                lowering: ctx.lowering_plan().clone(),
                amount: ctx.funds(),
                network: ctx.network,
                effects: unsafe { ctx.get_effects_internal() }.as_ref().clone(),
                ordinals_info: ctx.get_ordinals().clone(),
            },
            arguments: self.g.clone(),
        };
        self.v.clone().call(ctx.path(), &create_args)
    }
}

#[cfg(target_arch = "wasm32")]
REGISTER![Wrapper, "logo.png"];
