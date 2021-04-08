// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[deny(missing_docs)]
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::{Hash, HashEngine};
use bitcoin::util::bip32::*;
use sapio_ctv_emulator_trait::Clause;
pub use sapio_ctv_emulator_trait::{CTVAvailable, CTVEmulator, EmulatorError, NullEmulator};
use serde::de::DeserializeOwned;
use serde::Serialize;

use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};

use bitcoin::secp256k1::{All, Secp256k1};
use bitcoin::util::psbt::PartiallySignedTransaction;
use rand::Rng;
use sapio_base::CTVHash;
use std::sync::Arc;
const MAX_MSG: usize = 1_000_000;

pub mod connections;
mod msgs;
pub mod servers;

thread_local! {
    pub static SECP: Secp256k1<All> = Secp256k1::new();
}

/// Helper function to create an InvalidInput error from a &str
fn input_error<T>(s: &str) -> Result<T, std::io::Error> {
    Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, s))
}

/// Compute a derivation path from a sha256 hash.
///
/// Format is a bit peculiar, it's 9 u32's with the top bit as 0 (for unhardened
/// derivation). We take each u32 in the hash (big endian) and mask off the top bit.
/// Then we go over the 8 u32s and make a 8 bit u32 from the top bits.
///
/// This is because the ChildNumber is a enum u31 where the top bit is used to
/// indicate hardened or not, so we can't just do the simple thing.
fn hash_to_child_vec(h: Sha256) -> Vec<ChildNumber> {
    let a: [u8; 32] = h.into_inner();
    let b: [[u8; 4]; 8] = unsafe { std::mem::transmute(a) };
    let mut c: Vec<ChildNumber> = b
        .iter()
        // Note: We mask off the top bit. This removes 8 bits of entropy from the hash,
        // but we add it back in later.
        .map(|x| (u32::from_be_bytes(*x) << 1) >> 1)
        .map(ChildNumber::from)
        .collect();
    // Add a unique 9th path for the MSB's
    c.push(
        b.iter()
            .enumerate()
            .map(|(i, x)| (u32::from_be_bytes(*x) >> 31) << i)
            .sum::<u32>()
            .into(),
    );
    c
}
