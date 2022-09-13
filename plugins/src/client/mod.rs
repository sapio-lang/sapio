// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! module interface for clients

use super::*;
pub use api::*;

use ext::*;
use sapio::contract::Compiled;

pub mod api;
mod exports;
mod ext;
use exports::*;
pub mod plugin;
pub use plugin::Plugin;
