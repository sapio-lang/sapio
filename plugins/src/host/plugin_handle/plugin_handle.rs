// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;
use std::error::Error;
/// Generic plugin handle interface.
///
/// TODO: trait objects for being able to e.g. run plugins remotely.
pub trait PluginHandle {
    fn create(&self, c: &CreateArgs<String>) -> Result<Compiled, Box<dyn Error>>;
    fn get_api(&self) -> Result<serde_json::value::Value, Box<dyn Error>>;
    fn get_name(&self) -> Result<String, Box<dyn Error>>;
}
