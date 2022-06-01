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

use sapio_contrib::contracts::staked_signer::{Operational, Staker};
/// # Bonded Staker
type BondedStaker = Staker<Operational>;

REGISTER![BondedStaker, "logo.png"];
