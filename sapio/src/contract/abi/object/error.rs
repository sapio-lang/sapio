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
    /// Public covenant lowering inputs or key derivation failed.
    Covenant(sapio_base::covenant::CovenantError),
    /// An artifact violates the transaction binding invariants.
    InvalidArtifact(super::ArtifactError),
    /// The selected emulator differs from the policy used at compilation.
    CovenantPolicyMismatch {
        /// The contract whose covenant policy differs.
        path: sapio_base::serialization_helpers::SArc<sapio_base::effects::EffectPath>,
        /// The committed transaction template.
        template: bitcoin::hashes::sha256::Hash,
        /// The clause derived from the recorded public inputs.
        expected: Box<sapio_base::Clause>,
        /// The clause advertised by the selected signer or public plan.
        actual: Box<sapio_base::Clause>,
    },
    /// An auxiliary-input mapping does not match its template.
    InvalidInputMapping {
        /// The template whose input mapping is invalid.
        template: bitcoin::hashes::sha256::Hash,
        /// Explanation of the mapping error.
        reason: &'static str,
    },
    /// Known funding cannot satisfy the bound contract or transaction.
    InvalidFunding {
        /// The contract whose funding is invalid.
        path: sapio_base::serialization_helpers::SArc<sapio_base::effects::EffectPath>,
        /// The contract or auxiliary input being checked.
        outpoint: bitcoin::OutPoint,
        /// Explanation of the funding error.
        reason: String,
    },
    /// Transaction lookup or insertion failed its integrity checks.
    TxIndex(TxIndexError),
    /// An emulator failed or returned an invalid signing response.
    Emulator(EmulatorError),
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
            Self::Covenant(error) => Some(error),
            Self::InvalidArtifact(error) => Some(error),
            Self::Psbt(error) => Some(error),
            Self::TxIndex(error) => Some(error),
            Self::Emulator(error) => Some(error),
            _ => None,
        }
    }
}
impl From<sapio_base::covenant::CovenantError> for ObjectError {
    fn from(error: sapio_base::covenant::CovenantError) -> Self {
        Self::Covenant(error)
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
        ObjectError::Emulator(e)
    }
}
impl From<TxIndexError> for ObjectError {
    fn from(e: TxIndexError) -> Self {
        ObjectError::TxIndex(e)
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
            Self::Covenant(error) => error.fmt(f),
            Self::InvalidArtifact(error) => error.fmt(f),
            Self::CovenantPolicyMismatch { path, template, expected, actual } => write!(
                f,
                "covenant policy mismatch for contract {} template {template}: expected {expected}, selected {actual}",
                String::from(path.0.as_ref().clone()),
            ),
            Self::InvalidInputMapping { template, reason } => {
                write!(f, "invalid input mapping for template {template}: {reason}")
            }
            Self::Psbt(error) => write!(f, "invalid unsigned transaction: {error}"),
            Self::InvalidFunding {
                path,
                outpoint,
                reason,
            } => {
                write!(
                    f,
                    "invalid funding {outpoint} for contract {}: {reason}",
                    String::from(path.0.as_ref().clone())
                )
            }
            Self::TxIndex(error) => write!(f, "transaction index: {error}"),
            Self::Emulator(error) => write!(f, "signing: {error}"),
            _ => write!(f, "{:?}", self),
        }
    }
}
