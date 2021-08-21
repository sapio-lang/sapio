// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use batching_trait::{BatchingTraitVersion0_1_1, Payment};
#[deny(missing_docs)]
use sapio::contract::*;
use sapio::util::amountrange::*;
use sapio::*;
use sapio_base::timelocks::{AnyRelTimeLock, RelHeight};
use sapio_wasm_plugin::client::*;
use sapio_wasm_plugin::*;
use schemars::*;
use serde::*;
use std::collections::VecDeque;
use std::convert::{TryFrom, TryInto};

use crate::sapio_base::Clause;
use sapio_contrib::contracts::federated_sidechain::{FederatedPegIn, CanBeginRecovery};
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// Federated Peg
type FederatedPeg = FederatedPegIn<CanBeginRecovery>;

#[derive(JsonSchema, Deserialize)]
#[serde(transparent)]
struct Wrap(FederatedPeg);
impl From<Wrap> for FederatedPeg{
    fn from(w:Wrap) -> Self {
        w.0
    }
}


REGISTER![[FederatedPeg, Wrap], "logo.png"];
