// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use bitcoin::amount::Amount;
use bitcoin::consensus::encode::Encodable;
use bitcoin::hashes::sha256;
use bitcoin::hashes::Hash;

/// Any type which can generate a CTVHash. Allows some decoupling in the future if some types will
/// not be literal transactions.
/// TODO: Rename to something like Transaction Extension Features
pub trait CTVHash {
    /// Uses BIP-119 Logic to compute a CTV Hash
    fn get_ctv_hash(&self, input_index: u32) -> sha256::Hash;
    /// Gets the total amount a transaction creates in outputs.
    fn total_amount(&self) -> Amount;
}
impl CTVHash for bitcoin::Transaction {
    fn get_ctv_hash(&self, input_index: u32) -> sha256::Hash {
        let mut ctv_hash = sha256::Hash::engine();
        self.version.consensus_encode(&mut ctv_hash).unwrap();
        self.lock_time.consensus_encode(&mut ctv_hash).unwrap();
        // BIP-119 includes every serialized scriptSig when any is nonempty.
        // Omitting this field is only valid for the all-empty case.
        if self.input.iter().any(|input| !input.script_sig.is_empty()) {
            let mut scripts = sha256::Hash::engine();
            for input in &self.input {
                input.script_sig.consensus_encode(&mut scripts).unwrap();
            }
            sha256::Hash::from_engine(scripts)
                .to_byte_array()
                .consensus_encode(&mut ctv_hash)
                .unwrap();
        }
        (self.input.len() as u32)
            .consensus_encode(&mut ctv_hash)
            .unwrap();
        {
            let mut enc = sha256::Hash::engine();
            for seq in self.input.iter().map(|i| i.sequence) {
                seq.consensus_encode(&mut enc).unwrap();
            }
            sha256::Hash::from_engine(enc)
                .to_byte_array()
                .consensus_encode(&mut ctv_hash)
                .unwrap();
        }

        (self.output.len() as u32)
            .consensus_encode(&mut ctv_hash)
            .unwrap();

        {
            let mut enc = sha256::Hash::engine();
            for out in self.output.iter() {
                out.consensus_encode(&mut enc).unwrap();
            }
            sha256::Hash::from_engine(enc)
                .to_byte_array()
                .consensus_encode(&mut ctv_hash)
                .unwrap();
        }
        input_index.consensus_encode(&mut ctv_hash).unwrap();
        sha256::Hash::from_engine(ctv_hash)
    }

    fn total_amount(&self) -> Amount {
        self.output.iter().map(|output| output.value).sum()
    }
}
