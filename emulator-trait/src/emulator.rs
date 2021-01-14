use bitcoin::hashes::sha256;
use bitcoin::util::psbt::PartiallySignedTransaction;
pub use sapio_base::Clause;
use std::fmt;
use std::sync::Arc;
#[derive(Debug)]
pub enum EmulatorError {
    NetworkIssue(std::io::Error),
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

pub trait CTVEmulator: Sync + Send {
    fn get_signer_for(&self, h: sha256::Hash) -> Result<Clause, EmulatorError>;
    fn sign(
        &self,
        b: PartiallySignedTransaction,
    ) -> Result<PartiallySignedTransaction, EmulatorError>;
}

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
