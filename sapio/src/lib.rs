// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Sapio Compiler Core Crate. Sapio is used to create multi-transaction Bitcoin Smart Contracts.
#![cfg_attr(feature = "nightly", feature(associated_type_defaults))]
#![deny(missing_docs)]
extern crate serde;

#[macro_use]
pub mod contract;
pub mod template;
pub mod util;

pub use sapio_base;
