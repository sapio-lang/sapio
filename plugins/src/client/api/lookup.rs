// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

///! Utility struct for looking up and decoding modules
use super::*;

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq)]
/// # Lookup Parameters
/// - either using a hash key (exact); or
/// - name (user configured)
pub enum LookupFrom {
    /// # Provide the Hex Encoded Hash of the WASM Module
    HashKey(String),
    /// # Give a Configurable Name
    Name(String),
    /// # Get the currently executing module hash
    This,
}
impl LookupFrom {
    pub fn to_key(&self) -> Option<[u8; 32]> {
        match self {
            LookupFrom::HashKey(hash) => {
                let mut r = [0u8; 32];
                hex::decode_to_slice(hash, &mut r).ok()?;
                Some(r)
            }
            LookupFrom::Name(name) => lookup_module_name(name),
            LookupFrom::This => lookup_this_module_name(),
        }
    }
}
