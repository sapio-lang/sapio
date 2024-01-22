// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//!  Errors during object construction

pub use crate::contract::abi::studio::*;
use bitcoin::util::taproot::TaprootBuilderError;
use miniscript::*;
use sapio_base::{miniscript, txindex::TxIndexError};
use sapio_ctv_emulator_trait::EmulatorError;

/// Error types that can arise when constructing an Object
#[derive(Debug)]
pub enum ObjectError {
    /// The Error was due to Miniscript Policy
    MiniscriptPolicy(miniscript::policy::compiler::CompilerError),
    /// The Error was due to Miniscript
    Miniscript(miniscript::Error),
    /// Error Building Taproot Tree
    TaprootBulderError(TaprootBuilderError),
    /// Unknown Script Type
    UnknownScriptType(bitcoin::Script),
    /// OpReturn Too Long
    OpReturnTooLong,
    /// The Error was for an unknown/unhandled reason
    Custom(Box<dyn std::error::Error>),
}
impl std::error::Error for ObjectError {}
impl From<TaprootBuilderError> for ObjectError {
    fn from(e: TaprootBuilderError) -> ObjectError {
        ObjectError::TaprootBulderError(e)
    }
}
impl From<EmulatorError> for ObjectError {
    fn from(e: EmulatorError) -> Self {
        ObjectError::Custom(Box::new(e))
    }
}
impl From<TxIndexError> for ObjectError {
    fn from(e: TxIndexError) -> Self {
        ObjectError::Custom(Box::new(e))
    }
}

impl From<miniscript::policy::compiler::CompilerError> for ObjectError {
    fn from(v: miniscript::policy::compiler::CompilerError) -> Self {
        ObjectError::MiniscriptPolicy(v)
    }
}

impl From<miniscript::Error> for ObjectError {
    fn from(v: miniscript::Error) -> Self {
        ObjectError::Miniscript(v)
    }
}

impl std::fmt::Display for ObjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}
