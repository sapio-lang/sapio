use super::emulator::{CTVEmulator, NullEmulator};
use super::{Amount, Compilable, CompilationError, Compiled};
use std::rc::Rc;
/// Context type is not copyable/clonable externally
pub struct Context {
    /* TODO: Add Context Fields! */
    available_funds: Amount,
    emulator: NullEmulator,
}

impl Context {
    pub fn new(amount: Amount, emulator: Option<Rc<dyn CTVEmulator>>) -> Self {
        Context {
            available_funds: amount,
            emulator: NullEmulator(emulator),
        }
    }
    pub fn ctv_emulator(
        &self,
        b: bitcoin::hashes::sha256::Hash,
    ) -> Result<crate::clause::Clause, CompilationError> {
        self.emulator.get_signer_for(b)
    }
    pub fn compile<A: Compilable>(&self, a: A) -> Result<Compiled, CompilationError> {
        a.compile(&self)
    }
    // TODO: Fix
    pub fn with_amount(&self, amount: Amount) -> Result<Self, CompilationError> {
        if self.available_funds < amount {
            Err(CompilationError::OutOfFunds)
        } else {
            Ok(Context {
                available_funds: amount,
                emulator: self.emulator.clone(),
            })
        }
    }
    pub fn spend_amount(&mut self, amount: Amount) -> Result<(), CompilationError> {
        if self.available_funds < amount {
            Err(CompilationError::OutOfFunds)
        } else {
            self.available_funds -= amount;
            Ok(())
        }
    }

    pub fn add_amount(&mut self, amount: Amount) {
        self.available_funds += amount;
    }

    pub fn template(&self) -> crate::template::Builder {
        crate::template::Builder::new(Context {
            emulator: self.emulator.clone(),
            ..*self
        })
    }
}
