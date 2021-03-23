// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use bitcoin::consensus::encode::Encodable;
use bitcoin::hashes::sha256;
use bitcoin::hashes::Hash;
use bitcoin::util::amount::Amount;

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
        (self.input.len() as u32)
            .consensus_encode(&mut ctv_hash)
            .unwrap();
        {
            let mut enc = sha256::Hash::engine();
            for seq in self.input.iter().map(|i| i.sequence) {
                seq.consensus_encode(&mut enc).unwrap();
            }
            sha256::Hash::from_engine(enc)
                .into_inner()
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
                .into_inner()
                .consensus_encode(&mut ctv_hash)
                .unwrap();
        }
        input_index.consensus_encode(&mut ctv_hash).unwrap();
        sha256::Hash::from_engine(ctv_hash)
    }

    fn total_amount(&self) -> Amount {
        Amount::from_sat(self.output.iter().fold(0, |a, b| a + b.value))
    }
}
