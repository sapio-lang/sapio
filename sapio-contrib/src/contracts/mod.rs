// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A collection of contracts of varying quality and usefullness
use bitcoin::util::amount::CoinAmount;
use sapio_base::Clause;

use sapio::contract::*;
use sapio::*;
use schemars::*;
use serde::*;
use std::convert::TryInto;
pub mod basic_examples;
pub mod channel;
pub mod coin_pool;
pub mod derivatives;
pub mod dynamic;
pub mod federated_sidechain;
pub mod hodl_chicken;
pub mod readme_contracts;
pub mod tic_tac_toe;
pub mod treepay;
pub mod undo_send;
pub mod vault;
pub mod staked_signer;