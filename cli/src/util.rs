// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use bitcoin::consensus::deserialize;
use bitcoin::util::psbt::PartiallySignedTransaction;

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
