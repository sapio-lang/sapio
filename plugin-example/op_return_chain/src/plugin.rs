// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
#![deny(missing_docs)]
use sapio::*;
use sapio_contrib::contracts::op_return_chain::ChainReturn;
use sapio_wasm_plugin::client::*;
use sapio_wasm_plugin::*;
use schemars::*;
use serde::*;

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
struct Wrapped(ChainReturn);

impl From<Wrapped> for ChainReturn {
    fn from(w: Wrapped) -> Self {
        w.0
    }
}

REGISTER![[ChainReturn, Wrapped], "logo.png"];
