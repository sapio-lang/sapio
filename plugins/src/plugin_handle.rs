// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use crate::API;
use sapio::contract::CompilationError;
use sapio_base::effects::EffectPath;

/// Generic plugin handle interface.
///
/// TODO: trait objects for being able to e.g. run plugins remotely.
pub trait PluginHandle {
    type Input;
    type Output;
    fn call(&self, path: &EffectPath, c: &Self::Input) -> Result<Self::Output, CompilationError>;
    fn get_api(&self) -> Result<API<Self::Input, Self::Output>, CompilationError>;
    fn get_name(&self) -> Result<String, CompilationError>;
    fn get_logo(&self) -> Result<String, CompilationError>;
}
