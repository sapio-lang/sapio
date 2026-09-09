// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
#![deny(missing_docs)]

//! concrete emulators for CTV

use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::util::bip32::*;
use sapio_base::covenant::hash_to_child_vec;
use sapio_ctv_emulator_trait::Clause;
pub use sapio_ctv_emulator_trait::{CTVAvailable, CTVEmulator, EmulatorError, NullEmulator};

/// Default elapsed I/O allowance for one emulator request, including queued
/// client calls and idle server connections. This is not a CPU execution limit.
pub const DEFAULT_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub(crate) fn validate_request_timeout(timeout: std::time::Duration) -> std::io::Result<()> {
    if timeout.is_zero() || tokio::time::Instant::now().checked_add(timeout).is_none() {
        return Err(input_err(
            "Emulator request timeout must be positive and representable",
        ));
    }
    Ok(())
}

use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};

use bitcoin::secp256k1::{All, Secp256k1};
use bitcoin::util::psbt::PartiallySignedTransaction;

use sapio_base::CTVHash;
use std::sync::Arc;

pub mod connections;
mod msgs;
pub mod servers;
mod wire;

thread_local! {
    /// global SECP instance anyone can use
    pub static SECP: Secp256k1<All> = Secp256k1::new();
}

/// Helper function to create an InvalidInput error from a &str
fn input_error<T>(s: &str) -> Result<T, std::io::Error> {
    Err(input_err(s))
}
fn input_err(s: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, s)
}
