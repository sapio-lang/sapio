// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use bitcoin::bip32::{Xpriv, Xpub};
use bitcoin::secp256k1::Secp256k1;
use emulator_connect::servers::hd::HDOracleEmulator;
use std::io::{Error, ErrorKind};

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("Usage: emulator_server SEED_FILE LISTEN_ADDRESS");
        return Ok(());
    }
    if args.len() != 2 {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "Usage: emulator_server SEED_FILE LISTEN_ADDRESS",
        ));
    }
    let contents = tokio::fs::read(&args[0]).await?;
    let address = args[1]
        .to_str()
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "Listen address must be UTF-8"))?;
    let root = Xpriv::new_master(bitcoin::Network::Regtest, &contents).map_err(Error::other)?;
    let public = Xpub::from_priv(&Secp256k1::new(), &root);
    let listener = tokio::net::TcpListener::bind(address).await?;
    println!(
        "Running Oracle With Key: {} on {}",
        public,
        listener.local_addr()?
    );
    HDOracleEmulator::new(root).serve(listener).await
}
