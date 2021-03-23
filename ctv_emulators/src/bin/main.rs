// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use bitcoin::secp256k1::Secp256k1;
use bitcoin::util::bip32::*;
use emulator_connect::servers::hd::*;

use tokio;
use tokio::io::AsyncReadExt;
#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    let filename = std::env::args().nth(1).expect("No Seed File Provided");
    let mut file = tokio::fs::File::open(filename)
        .await
        .expect("File Not Found");
    let mut contents = vec![];
    file.read_to_end(&mut contents).await?;

    let root =
        ExtendedPrivKey::new_master(bitcoin::network::constants::Network::Regtest, &contents[..])
            .unwrap();
    let pk_root = ExtendedPubKey::from_private(&Secp256k1::new(), &root);
    let oracle = HDOracleEmulator::new(root, true);
    let server = oracle.bind(
        std::env::args()
            .nth(2)
            .expect("No Interface given (e.g., 127.0.0.1:8080"),
    );
    println!("Running Oracle With Key: {}", pk_root);
    server.await?;
    Ok(())
}
