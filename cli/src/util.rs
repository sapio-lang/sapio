// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use bitcoin::consensus::deserialize;
use bitcoin::hashes::Hash;
use bitcoin::util::psbt::PartiallySignedTransaction;
use std::path::PathBuf;
/// Checks that a file exists during argument parsing
///
/// **Race Conditions** if file is deleted after this call
pub fn check_file(p: &str) -> Result<(), String> {
    std::fs::metadata(p).map_err(|_| String::from("File doesn't exist"))?;
    Ok(())
}
/// Checks that a file does not exist during argument parsing
///
/// **Race Conditions** if file is created after this call
pub fn check_file_not(p: &str) -> Result<(), String> {
    if std::fs::metadata(p).is_ok() {
        return Err(String::from("File exists already"));
    }
    Ok(())
}

/// Reads a PSBT from a file and checks that it is correctly formatted
pub fn decode_psbt_file(
    a: &clap::ArgMatches,
    b: &str,
) -> Result<PartiallySignedTransaction, Box<dyn std::error::Error>> {
    let bytes = std::fs::read_to_string(a.value_of_os(b).unwrap())?;
    let bytes = base64::decode(&bytes.trim()[..])?;
    let psbt: PartiallySignedTransaction = deserialize(&bytes[..])?;
    Ok(psbt)
}



/// get the path for the compiled modules
pub(crate) fn get_data_dir(typ: &str, org: &str, proj: &str) -> PathBuf {
    let proj =
        directories::ProjectDirs::from(typ, org, proj).expect("Failed to find config directory");
    let path: PathBuf = proj.data_dir().clone().into();
    path
}

pub(crate) fn create_mock_output() -> bitcoin::OutPoint {
    bitcoin::OutPoint {
        txid: bitcoin::hashes::sha256d::Hash::from_inner(
            bitcoin::hashes::sha256::Hash::hash(format!("mock:{}", 0).as_bytes()).into_inner(),
        )
        .into(),
        vout: 0,
    }
}
