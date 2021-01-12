use super::*;
pub struct FederatedEmulatorConnection {
    emulators: Vec<Box<dyn CTVEmulator>>,
    threshold: u8,
}

impl FederatedEmulatorConnection {
    pub fn new(emulators: Vec<Box<dyn CTVEmulator>>, threshold: u8) -> Self {
        FederatedEmulatorConnection {
            emulators,
            threshold,
        }
    }
}

impl CTVEmulator for FederatedEmulatorConnection {
    fn get_signer_for(&self, h: Sha256) -> Result<Clause, EmulatorError> {
        let v = self
            .emulators
            .iter()
            .map(|e| e.get_signer_for(h))
            .collect::<Result<Vec<Clause>, EmulatorError>>()?;
        Ok(Clause::Threshold(self.threshold as usize, v))
    }
    fn sign(
        &self,
        mut b: PartiallySignedTransaction,
    ) -> Result<PartiallySignedTransaction, EmulatorError> {
        for emulator in self.emulators.iter() {
            b = emulator.sign(b)?;
        }
        Ok(b)
    }
}
