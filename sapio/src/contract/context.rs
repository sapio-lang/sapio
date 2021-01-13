use super::{Amount, Compilable, CompilationError, Compiled};
use crate::util::amountrange::AmountRange;
use bitcoin::Network;
use emulator_connect::{CTVEmulator, NullEmulator};
use miniscript::Descriptor;
use std::collections::HashMap;
use std::sync::Arc;
/// Context type is not copyable/clonable externally
pub struct Context {
    /* TODO: Add Context Fields! */
    available_funds: Amount,
    emulator: NullEmulator,
    pub network: Network,
}

impl Context {
    pub fn new(network: Network, amount: Amount, emulator: Option<Arc<dyn CTVEmulator>>) -> Self {
        Context {
            available_funds: amount,
            emulator: NullEmulator(emulator),
            network,
        }
    }
    pub fn funds(&self) -> Amount {
        self.available_funds
    }
    pub fn ctv_emulator(
        &self,
        b: bitcoin::hashes::sha256::Hash,
    ) -> Result<sapio_base::Clause, CompilationError> {
        Ok(self.emulator.get_signer_for(b)?)
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
                ..*self
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

    /// converts a descriptor and an optional AmountRange to a Object object.
    /// This can be used for e.g. creating raw SegWit Scripts.
    pub fn compiled_from_descriptor(
        d: Descriptor<bitcoin::PublicKey>,
        a: Option<AmountRange>,
    ) -> Compiled {
        Compiled {
            ctv_to_tx: HashMap::new(),
            suggested_txs: HashMap::new(),
            policy: None,
            address: d.address(bitcoin::Network::Bitcoin).unwrap(),
            descriptor: Some(d),
            amount_range: a.unwrap_or_else(|| {
                let mut a = AmountRange::new();
                a.update_range(Amount::min_value());
                a.update_range(Amount::max_value());
                a
            }),
        }
    }
}
