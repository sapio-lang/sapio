// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Utilities for working with SIMPs (Sapio Interactive Metadata Protocols)
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

/// Errors that may come up when working with SIMPs
#[derive(Debug)]
pub enum SIMPError {
    /// If this SIMP is already present.
    /// Implementors may wish to handle or ignore this error if it is not an
    /// issue, but usually it is a bug.
    /// todo: Mergeable SIMPs may merge one another
    AlreadyDefined(serde_json::Value),
    /// If the error was because a SIMP could not be serialized.
    ///
    /// If this error ever happens, your SIMP is poorly designed most likely!
    SerializationError(serde_json::Error),
}
impl std::fmt::Display for SIMPError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for SIMPError {}
impl From<serde_json::Error> for SIMPError {
    fn from(v: serde_json::Error) -> Self {
        SIMPError::SerializationError(v)
    }
}

/// Trait for Sapio Interactive Metadata Protocol Implementors
pub trait SIMP {
    /// Get a protocol number, which should be one that is assigned through the
    /// SIMP repo. Proprietary SIMPs can safely use negative numbers.
    fn get_protocol_number(&self) -> i64;
    fn to_json(&self) -> Result<Value, serde_json::Error>;
    fn from_json(value: Value) -> Result<Self, serde_json::Error>
    where
        Self: Sized;
}

pub trait LocationTag {}

macro_rules! gen_location {
    ($x:ident) => {
        pub struct $x;
        impl LocationTag for $x {}
    };
}

gen_location!(ContinuationPointLT);
gen_location!(CompiledObjectLT);
gen_location!(TemplateLT);
gen_location!(TemplateOutputLT);
gen_location!(GuardLT);
gen_location!(TemplateInputLT);

pub trait SIMPAttachableAt<T: LocationTag>
where
    Self: SIMP,
{
}
