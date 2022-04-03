// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;
use sapio::contract::CompilationError;
use sapio_base::effects::EffectPath;
/// Generic plugin handle interface.
///
/// TODO: trait objects for being able to e.g. run plugins remotely.
pub trait PluginHandle {
    fn create(
        &self,
        path: &EffectPath,
        c: &CreateArgs<serde_json::Value>,
    ) -> Result<Compiled, CompilationError>;
    fn get_api(&self) -> Result<serde_json::value::Value, CompilationError>;
    fn get_name(&self) -> Result<String, CompilationError>;
    fn get_logo(&self) -> Result<String, CompilationError>;
}
