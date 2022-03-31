// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Utilities for working with SIMPs (Sapio Interactive Metadata Protocols)
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

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
pub trait SIMP: Serialize + for<'de> Deserialize<'de> + JsonSchema {
    /// Get a protocol number, which should be one that is assigned through the
    /// SIMP repo. Proprietary SIMPs can safely use negative numbers.
    fn get_protocol_number() -> i64;
}
