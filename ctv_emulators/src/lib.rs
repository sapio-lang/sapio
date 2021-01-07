use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::{Hash, HashEngine};
use bitcoin::util::bip32::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
pub mod emulator;
use emulator::Clause;
pub use emulator::{CTVEmulator, EmulatorError, NullEmulator};

use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};

use bitcoin::consensus::encode::{Decodable, Encodable};
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

fn input_error<T>(s: &str) -> Result<T, std::io::Error> {
    Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, s))
}

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
