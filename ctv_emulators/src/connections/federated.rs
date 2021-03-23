// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;
/// Creates a multi-condition emulator with a certain threshold.
/// It implements CTVEmulator so that it itself can be used as a trait object.
pub struct FederatedEmulatorConnection {
    emulators: Vec<Arc<dyn CTVEmulator>>,
    threshold: u8,
}

impl FederatedEmulatorConnection {
    pub fn new(emulators: Vec<Arc<dyn CTVEmulator>>, threshold: u8) -> Self {
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
