// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[deny(missing_docs)]
use sapio_wasm_plugin::client::*;
use sapio_wasm_plugin::*;
use schemars::*;
use serde::*;

use sapio_contrib::contracts::hanukkah::Hanukkiah2;

#[derive(JsonSchema, Deserialize)]
#[serde(transparent)]
struct Wrap(Hanukkiah2);
impl From<Wrap> for Hanukkiah2 {
    fn from(w: Wrap) -> Self {
        w.0
    }
}

REGISTER![[Hanukkiah2, Wrap], "logo.png"];
