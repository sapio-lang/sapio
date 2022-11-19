// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![deny(missing_docs)]
//! module interfaces for sapio clients and hosts
use sapio::contract::{Compilable, Context};
pub use sapio_base::plugin_args::*;
use sapio_data_repr::{HasSapioModuleSchema, SapioModuleSchema};
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
#[derive(Serialize, Deserialize)]
pub struct API<Input, Output> {
    /// What is expected to be passed to the module
    arguments: SapioModuleSchema,
    /// What is expected to be returned from the module
    returns: SapioModuleSchema,
    #[serde(skip, default)]
    _pd: PhantomData<(Input, Output)>,
}
impl<Input: HasSapioModuleSchema, Output: HasSapioModuleSchema> HasSapioModuleSchema
    for API<Input, Output>
{
    fn get_schema() -> SapioModuleSchema {
        todo!()
    }
}

impl<Input, Output> API<Input, Output>
where
    Input: HasSapioModuleSchema,
    Output: HasSapioModuleSchema,
{
    /// Create a new API for this type with freshly generated schemas
    pub fn new() -> Self {
        API {
            arguments: todo!(),
            returns: todo!(),
            _pd: Default::default(),
        }
    }
    /// get the input schema for this type as a reference
    pub fn input(&self) -> &SapioModuleSchema {
        &self.arguments
    }
    /// get the output schema for this type as a reference
    pub fn output(&self) -> &SapioModuleSchema {
        &self.returns
    }
}
