use super::{CompilationError};
use crate::clause::Clause;
use bitcoin::hashes::sha256;
use std::rc::Rc;
pub trait CTVEmulator {
    fn get_signer_for(&self, h: sha256::Hash) -> Result<Clause, CompilationError>;
    fn sign(&self, b: bitcoin::util::psbt::PartiallySignedTransaction) -> bitcoin::util::psbt::PartiallySignedTransaction;
}

#[derive(Clone)]
pub(crate) struct NullEmulator(pub(crate) Option<Rc<dyn CTVEmulator>>);

impl CTVEmulator for NullEmulator {
    fn get_signer_for(&self, h: sha256::Hash) -> Result<Clause, CompilationError> {
        match &self.0 {
            None => Ok(Clause::TxTemplate(h)),
            Some(emulator) => emulator.get_signer_for(h)
        }
    }
    fn sign(&self, b: bitcoin::util::psbt::PartiallySignedTransaction) -> bitcoin::util::psbt::PartiallySignedTransaction {
        b
    }
}