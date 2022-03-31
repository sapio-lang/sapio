// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use sapio_base::simp::SIMP;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
/// A URL to a project for convenience
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct URL {
    #[schemars(url)]
    pub url: String,
}
/// An IPFS Based NFT Spec
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct IpfsNFT {
    /// The Content ID to be retrieved through IPFS
    pub cid: String,
    /// If the NFT is one of a series, it is first / second
    pub edition: Option<(u64, u64)>,
    /// The Artist's Public Key
    #[serde(flatten)]
    // TODO: fixup representation with patches to add more Schemars to bitcoin
    #[schemars(with = "Option::<String>")]
    pub artist: Option<bitcoin::secp256k1::XOnlyPublicKey>,
    /// The signature of artist
    #[serde(flatten)]
    // TODO: fixup representation with patches to add more Schemars to bitcoin
    #[schemars(with = "Option::<String>")]
    pub blessing: Option<bitcoin::secp256k1::schnorr::Signature>,
    /// If the NFT has a webpage (legacy web)
    #[serde(flatten)]
    pub softlink: Option<URL>,
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
