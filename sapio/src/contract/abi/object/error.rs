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
    /// An artifact violates the transaction binding invariants.
    InvalidArtifact(super::ArtifactError),
    /// An auxiliary-input mapping does not match its template.
    InvalidInputMapping {
        /// The template whose input mapping is invalid.
        template: bitcoin::hashes::sha256::Hash,
        /// Explanation of the mapping error.
        reason: &'static str,
    },
    /// The transaction cannot be represented as an unsigned PSBT.
    Psbt(bitcoin::util::psbt::Error),
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
impl std::error::Error for ObjectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidArtifact(error) => Some(error),
            Self::Psbt(error) => Some(error),
            _ => None,
        }
    }
}
impl From<super::ArtifactError> for ObjectError {
    fn from(error: super::ArtifactError) -> Self {
        Self::InvalidArtifact(error)
    }
}
impl From<bitcoin::util::psbt::Error> for ObjectError {
    fn from(error: bitcoin::util::psbt::Error) -> Self {
        Self::Psbt(error)
    }
}
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
        match self {
            Self::InvalidArtifact(error) => error.fmt(f),
            Self::InvalidInputMapping { template, reason } => {
                write!(f, "invalid input mapping for template {template}: {reason}")
            }
            Self::Psbt(error) => write!(f, "invalid unsigned transaction: {error}"),
            _ => write!(f, "{:?}", self),
        }
    }
}
