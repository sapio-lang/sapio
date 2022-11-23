// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![deny(missing_docs)]
//! module interfaces for sapio clients and hosts
use sapio::contract::{Compilable, Context};
pub use sapio_base::plugin_args::*;
use sapio_data_repr::{ReprSpec, ReprSpecifiable};
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
    arguments: ReprSpec,
    /// What is expected to be returned from the module
    returns: ReprSpec,
    #[serde(skip, default)]
    _pd: PhantomData<(Input, Output)>,
}
impl<Input: ReprSpecifiable, Output: ReprSpecifiable> ReprSpecifiable for API<Input, Output> {
    fn get_repr_spec() -> ReprSpec {
        todo!()
    }
}

impl<Input, Output> API<Input, Output>
where
    Input: ReprSpecifiable,
    Output: ReprSpecifiable,
{
    /// Create a new API for this type with freshly generated schemas
    pub fn new() -> Self {
        API {
            arguments: Input::get_repr_spec(),
            returns: Output::get_repr_spec(),
            _pd: Default::default(),
        }
    }
    /// get the input schema for this type as a reference
    pub fn input(&self) -> &ReprSpec {
        &self.arguments
    }
    /// get the output schema for this type as a reference
    pub fn output(&self) -> &ReprSpec {
        &self.returns
    }
}
