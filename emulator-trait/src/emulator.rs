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
/// Errors that an emulator might throw
#[derive(Debug)]
pub enum EmulatorError {
    /// Wraps an issue caused in a Network/IO context
    /// (TODO: Prevents serialization/deserialization)
    NetworkIssue(std::io::Error),
    /// Error was caused by BIP32
    BIP32Error(bitcoin::util::bip32::Error),
    /// A signer changed or removed PSBT data instead of only adding signatures.
    InvalidResponse,
}
impl fmt::Display for EmulatorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EmulatorError::InvalidResponse => {
                f.write_str("Emulator response must preserve the PSBT and only add signatures")
            }
            _ => write!(f, "{:?}", self),
        }
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
    /// Emulator would satisfy. This must be deterministic for the backend's
    /// configuration. Binding compares it against the artifact's public
    /// lowering plan before funding or signing. Compilation never calls this
    /// method. Endpoints and transport settings must not change the policy.
    fn get_signer_for(&self, h: sha256::Hash) -> Result<Clause, EmulatorError>;
    /// Returns the complete PSBT with the emulator's signatures added, if any.
    /// Existing signatures and all other fields must remain unchanged.
    /// Call [`sign_checked`] when accepting a response from an emulator.
    fn sign(
        &self,
        b: PartiallySignedTransaction,
    ) -> Result<PartiallySignedTransaction, EmulatorError>;
}

/// Call an emulator and enforce the complete-response signing contract.
///
/// Only additions to ECDSA partial signatures, Taproot key signatures and
/// Taproot script signatures are permitted. This function checks response
/// integrity; signature validity must still be checked during finalization.
pub fn sign_checked(
    emulator: &dyn CTVEmulator,
    request: PartiallySignedTransaction,
) -> Result<PartiallySignedTransaction, EmulatorError> {
    let response = emulator.sign(request.clone())?;
    validate_signing_response(&request, &response)?;
    Ok(response)
}

/// Check that a complete signer response only adds signatures to a PSBT.
///
/// Existing signatures cannot be replaced or removed. Every other field,
/// including unknown metadata and finalized scripts, must remain unchanged.
/// This does not verify new signatures or the validity of the original PSBT.
pub fn validate_signing_response(
    request: &PartiallySignedTransaction,
    response: &PartiallySignedTransaction,
) -> Result<(), EmulatorError> {
    if request.inputs.len() != response.inputs.len()
        || request.outputs.len() != response.outputs.len()
        || response.inputs.len() != response.unsigned_tx.input.len()
        || response.outputs.len() != response.unsigned_tx.output.len()
    {
        return Err(EmulatorError::InvalidResponse);
    }
    let mut preserved = response.clone();
    for (original, changed) in request.inputs.iter().zip(&mut preserved.inputs) {
        if original
            .partial_sigs
            .iter()
            .any(|(key, signature)| changed.partial_sigs.get(key) != Some(signature))
            || original
                .tap_script_sigs
                .iter()
                .any(|(key, signature)| changed.tap_script_sigs.get(key) != Some(signature))
            || (original.tap_key_sig.is_some() && changed.tap_key_sig != original.tap_key_sig)
        {
            return Err(EmulatorError::InvalidResponse);
        }
        // Compare the whole PSBT after masking only the permitted additions.
        // New non-signature fields remain protected by structural equality.
        changed.partial_sigs.clone_from(&original.partial_sigs);
        changed.tap_key_sig = original.tap_key_sig;
        changed
            .tap_script_sigs
            .clone_from(&original.tap_script_sigs);
    }
    if preserved != *request {
        return Err(EmulatorError::InvalidResponse);
    }
    Ok(())
}

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
