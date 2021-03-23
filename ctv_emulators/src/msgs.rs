// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;
use bitcoin::consensus::encode::{Decodable, Encodable};
use miniscript::serde;
use serde::de::Visitor;
use serde::de::*;
use serde::*;
use std::fmt;

const MAX_MSG: usize = 1_000_000;

/// a PSBT Wrapper type. Note that Serialize/Deserialize are manually implemented
/// limited to 1MB in size.
#[derive(Clone)]
pub struct PSBT(pub PartiallySignedTransaction);

/// A message for a client to challenge a server to prove it has the key
#[derive(Serialize, Deserialize)]
pub struct ConfirmKey(pub ExtendedPubKey, pub Sha256);

/// a response from a server to a client with a challenge response
#[derive(Serialize, Deserialize)]
pub struct KeyConfirmed(pub bitcoin::secp256k1::Signature, pub Sha256);

/// Wrapper for message serialization
#[derive(Serialize, Deserialize)]
pub enum Request {
    ConfirmKey(ConfirmKey),
    SignPSBT(PSBT),
}

/// A visitor tage for a SafePSBT type that is size limited
/// Serialized/deserialized with a size tag internally.
struct SafePSBT(usize);

impl<'de> Visitor<'de> for SafePSBT {
    type Value = PSBT;
    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(&format!(
            "Expecting a PSBT serialized smaller than {}",
            self.0
        ))
    }
    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A::Error: de::Error,
        A: SeqAccess<'de>,
    {
        let length_error =
            || de::Error::invalid_length(self.0 as usize, &"Expected at least 4 bytes.");
        let len: usize = u32::from_be_bytes([
            seq.next_element()?.ok_or_else(length_error)?,
            seq.next_element()?.ok_or_else(length_error)?,
            seq.next_element()?.ok_or_else(length_error)?,
            seq.next_element()?.ok_or_else(length_error)?,
        ]) as usize;
        let length_error = de::Error::invalid_length(self.0, &"Length Exceeded Maximum");
        if len > self.0 {
            return Err(length_error);
        }

        let mut v = vec![];
        let length_error = || de::Error::invalid_length(len, &"Expected enough bytes");
        for _ in 0..len {
            v.push(seq.next_element()?.ok_or_else(length_error)?);
        }

        return PartiallySignedTransaction::consensus_decode(&v[..])
            .map_err(de::Error::custom)
            .map(PSBT);
    }
}

impl Serialize for PSBT {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut m = vec![0u8; 4];
        self.0
            .consensus_encode(&mut m)
            .map_err(ser::Error::custom)?;
        let len = m.len();
        m[..4].copy_from_slice(&((len - 4) as u32).to_be_bytes()[..]);
        serializer.serialize_bytes(&m)
    }
}

impl<'de> Deserialize<'de> for PSBT {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        d.deserialize_bytes(SafePSBT(MAX_MSG))
    }
}
