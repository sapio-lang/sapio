// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! generic plugin handle interface available to client and host

use crate::API;
use sapio::contract::CompilationError;
use sapio_base::effects::EffectPath;

/// Generic plugin handle interface.
///
// TODO: trait objects for being able to e.g. run plugins remotely.
pub trait PluginHandle {
    /// The object type a module recieves
    type Input;
    /// The object type a module outputs
    type Output;
    /// Call the module's main function
    fn call(&self, path: &EffectPath, c: &Self::Input) -> Result<Self::Output, CompilationError>;
    /// get api metadata
    fn get_api(&self) -> Result<API<Self::Input, Self::Output>, CompilationError>;
    /// get name metadata
    fn get_name(&self) -> Result<String, CompilationError>;
    /// get logo metadata
    fn get_logo(&self) -> Result<String, CompilationError>;
}
