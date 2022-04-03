// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use sapio_base::simp::SIMP;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
/// A URL to a project for convenience
#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub struct URL {
    #[schemars(url)]
    pub url: String,
}
/// An IPFS Based NFT Spec
#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub struct IpfsNFT {
    /// The Content ID to be retrieved through IPFS
    pub cid: String,
    /// The NFT version, for extensibility. Must be 0 as of now.
    pub version: u64,
    /// If the NFT is one of a series, which number.
    pub edition: u64,
    /// If the NFT is one of a series, out of how many
    pub of_edition_count: u64,
    /// The Artist's Public Key
    // TODO: fixup representation with patches to add more Schemars to bitcoin
    #[schemars(with = "Option::<String>")]
    pub artist: Option<bitcoin::secp256k1::XOnlyPublicKey>,
    /// The signature of artist
    // TODO: fixup representation with patches to add more Schemars to bitcoin
    #[schemars(with = "Option::<String>")]
    pub blessing: Option<bitcoin::secp256k1::schnorr::Signature>,
    /// If the NFT has a webpage (legacy web)
    pub softlink: Option<URL>,
}
use bitcoin::hashes::sha256::Hash as sha256;
use bitcoin::hashes::sha256::HashEngine as engine;
use bitcoin::hashes::Hash;
use bitcoin::hashes::HashEngine;
impl IpfsNFT {
    /// Canonicalized commitment to IpfsNFT data
    pub fn commitment(&self) -> sha256 {
        let h1 = sha256::hash(self.cid.as_bytes()).into_inner();
        let artist = self.artist.map(|b| b.serialize()).unwrap_or([0u8; 32]);
        let blessing = self
            .blessing
            .map(|b| b.as_ref().clone())
            .unwrap_or([0u8; 64]);
        let softlink = self
            .softlink
            .as_ref()
            .map(|s| sha256::hash(s.url.as_bytes()).into_inner())
            .unwrap_or([0u8; 32]);
        let mut eng = engine::default();
        eng.input(&self.version.to_be_bytes());
        eng.input(&h1);
        eng.input(&self.edition.to_be_bytes());
        eng.input(&self.of_edition_count.to_be_bytes());
        eng.input(&artist);
        eng.input(&blessing);
        eng.input(&softlink);
        sha256::from_engine(eng)
    }
}
impl SIMP for IpfsNFT {
    fn get_protocol_number() -> i64 {
        -12345
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}
