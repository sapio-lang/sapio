// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![deny(missing_docs)]

//! Staker Contract

#[cfg(target_arch = "wasm32")]
use sapio_wasm_plugin::{optional_logo, REGISTER};

#[cfg(target_arch = "wasm32")]
use sapio_contrib::contracts::staked_signer::{Operational, Staker};
/// # Bonded Staker
#[cfg(target_arch = "wasm32")]
type BondedStaker = Staker<Operational>;

#[cfg(target_arch = "wasm32")]
REGISTER![BondedStaker, "logo.png"];
