// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! definitions of emulator traits required to use as a trait object in low-level libraries.
use bitcoin::hashes::sha256;
use bitcoin::util::psbt::PartiallySignedTransaction;
pub use sapio_base::Clause;
use std::fmt;
use std::sync::Arc;
/// Errors that an emulator might throw
#[derive(Debug)]
pub enum EmulatorError {
    /// Wraps an issue caused in a Network/IO context
    /// (TODO: Prevents serialization/deserialization)
    NetworkIssue(std::io::Error),
    /// Error was caused by BIP32
    BIP32Error(bitcoin::util::bip32::Error),
}
impl fmt::Display for EmulatorError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
impl std::error::Error for EmulatorError {}

impl From<std::io::Error> for EmulatorError {
    fn from(e: std::io::Error) -> EmulatorError {
        EmulatorError::NetworkIssue(e)
    }
}

impl From<bitcoin::util::bip32::Error> for EmulatorError {
    fn from(e: bitcoin::util::bip32::Error) -> EmulatorError {
        EmulatorError::BIP32Error(e)
    }
}

/// `CTVEmulator` trait is used to make the method in which CheckTemplateVerify
/// is stubbed out with.
pub trait CTVEmulator: Sync + Send {
    /// For a given transaction hash, gets the corresponding Clause that the
    /// Emulator would satisfy.
    fn get_signer_for(&self, h: sha256::Hash) -> Result<Clause, EmulatorError>;
    /// Adds the Emulators signature to the PSBT, if any.
    fn sign(
        &self,
        b: PartiallySignedTransaction,
    ) -> Result<PartiallySignedTransaction, EmulatorError>;
}

/// A wrapper for an optional internal emulator trait object. If no emulator is
/// provided, then it defaults to using actual CheckTemplateVerify Clauses.
pub type NullEmulator = Arc<dyn CTVEmulator>;

/// a type tag that can be tossed inside an Arc to get CTV
pub struct CTVAvailable;
impl CTVEmulator for CTVAvailable {
    fn get_signer_for(&self, h: sha256::Hash) -> Result<Clause, EmulatorError> {
        Ok(Clause::TxTemplate(h))
    }
    fn sign(
        &self,
        b: PartiallySignedTransaction,
    ) -> Result<PartiallySignedTransaction, EmulatorError> {
        Ok(b)
    }
}
