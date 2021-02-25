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
#[derive(Clone)]
pub struct NullEmulator(pub Option<Arc<dyn CTVEmulator>>);

impl CTVEmulator for NullEmulator {
    fn get_signer_for(&self, h: sha256::Hash) -> Result<Clause, EmulatorError> {
        match &self.0 {
            None => Ok(Clause::TxTemplate(h)),
            Some(emulator) => emulator.get_signer_for(h),
        }
    }
    fn sign(
        &self,
        b: PartiallySignedTransaction,
    ) -> Result<PartiallySignedTransaction, EmulatorError> {
        match &self.0 {
            None => Ok(b),
            Some(emulator) => emulator.sign(b),
        }
    }
}
