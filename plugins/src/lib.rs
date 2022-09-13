// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![deny(missing_docs)]
//! module interfaces for sapio clients and hosts
use sapio::contract::{Compilable, Context};
pub use sapio_base::plugin_args::*;
use schemars::schema::RootSchema;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::ffi::CString;
use std::marker::PhantomData;
use std::os::raw::c_char;
use std::sync::Arc;

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "host")]
pub mod host;
pub mod plugin_handle;

/// A bundle of input/output types
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct API<Input, Output> {
    /// What is expected to be passed to the module
    arguments: RootSchema,
    /// What is expected to be returned from the module
    returns: RootSchema,
    #[serde(skip, default)]
    _pd: PhantomData<(Input, Output)>,
}
impl<Input, Output> API<Input, Output>
where
    Input: JsonSchema,
    Output: JsonSchema,
{
    /// Create a new API for this type with freshly generated schemas
    pub fn new() -> Self {
        API {
            arguments: schemars::schema_for!(Input),
            returns: schemars::schema_for!(Output),
            _pd: Default::default(),
        }
    }
    /// get the input schema for this type as a reference
    pub fn input(&self) -> &RootSchema {
        &self.arguments
    }
    /// get the output schema for this type as a reference
    pub fn output(&self) -> &RootSchema {
        &self.returns
    }
}
