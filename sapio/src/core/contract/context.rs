use super::{Amount, CompilationError, Compilable, Compiled};
#[derive(Clone)]
pub struct Context {
    /* TODO: Add Context Fields! */
    available_funds: Amount,
}

impl Context {
    pub fn new(amount: Amount) -> Self {
        Context {
            available_funds: amount,
        }
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
                ..self.clone()
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
        crate::template::Builder::new(self.clone())
    }
}
