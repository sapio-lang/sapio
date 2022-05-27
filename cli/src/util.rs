// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use bitcoin::consensus::deserialize;
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

use bitcoin::secp256k1::{All, Secp256k1};
use bitcoin::util::bip32::ExtendedPrivKey;
use bitcoin::util::sighash::Prevouts;
use bitcoin::util::taproot::TapLeafHash;
use bitcoin::util::taproot::TapSighashHash;
use bitcoin::SchnorrSig;
use bitcoin::Script;
use bitcoin::TxOut;
use bitcoin::XOnlyPublicKey;
pub fn sign_psbt(
    xpriv: &ExtendedPrivKey,
    mut b: PartiallySignedTransaction,
    secp: &Secp256k1<All>,
) -> Result<PartiallySignedTransaction, std::io::Error> {
    let tx = b.clone().extract_tx();
    let utxos: Vec<TxOut> = b
        .inputs
        .iter()
        .map(|o| o.witness_utxo.clone())
        .collect::<Option<Vec<TxOut>>>()
        .ok_or_else(|| input_err("Could not find one of the UTXOs to be signed over"))?;
    let untweaked = xpriv.to_keypair(secp);
    let pk = XOnlyPublicKey::from_keypair(&untweaked);
    let mut sighash = bitcoin::util::sighash::SigHashCache::new(&tx);
    let input_zero = &mut b.inputs[0];
    use bitcoin::schnorr::TapTweak;
    let tweaked = untweaked
        .tap_tweak(secp, input_zero.tap_merkle_root)
        .into_inner();
    let tweaked_pk = tweaked.public_key();
    let hash_ty = bitcoin::util::sighash::SchnorrSigHashType::All;
    let prevouts = &Prevouts::All(&utxos);
    let mut get_sig = |path, kp| {
        let annex = None;
        let sighash: TapSighashHash = sighash
            .taproot_signature_hash(0, prevouts, annex, path, hash_ty)
            .expect("Signature hash cannot fail...");
        let msg =
            bitcoin::secp256k1::Message::from_slice(&sighash[..]).expect("Size must be correct.");
        let sig = secp.sign_schnorr_no_aux_rand(&msg, kp);
        SchnorrSig { sig, hash_ty }
    };
    if input_zero.tap_internal_key == Some(pk) {
        let sig = get_sig(None, &tweaked);
        input_zero.tap_key_sig = Some(sig);
    }
    for tlh in input_zero
        .tap_scripts
        .values()
        .map(|(script, ver)| TapLeafHash::from_script(script, *ver))
    {
        let sig = get_sig(Some((tlh, 0xffffffff)), &untweaked);
        input_zero.tap_script_sigs.insert((pk.clone(), tlh), sig);
    }
    Ok(b)
}

fn input_err(s: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, s)
}

/// get the path for the compiled modules
pub(crate) fn get_path(typ: &str, org: &str, proj: &str) -> PathBuf {
    let proj =
        directories::ProjectDirs::from(typ, org, proj).expect("Failed to find config directory");
    let mut path: PathBuf = proj.data_dir().clone().into();
    path.push("modules");
    path
}
