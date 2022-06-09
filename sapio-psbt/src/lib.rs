// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use bitcoin::consensus::serialize;
use bitcoin::secp256k1::rand::Rng;
use bitcoin::secp256k1::{rand, All};
use bitcoin::util::bip32::{ExtendedPubKey, KeySource};
use bitcoin::util::sighash::Prevouts;
use bitcoin::util::taproot::TapLeafHash;
use bitcoin::util::taproot::TapSighashHash;
use bitcoin::XOnlyPublicKey;
use bitcoin::{
    psbt::PartiallySignedTransaction, secp256k1::Secp256k1, util::bip32::ExtendedPrivKey,
};
use bitcoin::{KeyPair, TxOut};
use bitcoin::{Network, SchnorrSig};
use miniscript::psbt::PsbtExt;
use miniscript::Tap;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::error::Error;
use std::str::FromStr;

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum PSBTApi {
    Finished {
        completed: bool,
        hex: String,
    },
    NotFinished {
        completed: bool,
        psbt: String,
        error: String,
        errors: Vec<String>,
    },
}

pub fn finalize_psbt_format_api(psbt: PartiallySignedTransaction) -> PSBTApi {
    let secp = Secp256k1::new();
    psbt.finalize(&secp)
        .map(|tx| {
            let hex = bitcoin::consensus::encode::serialize_hex(&tx.extract_tx());
            PSBTApi::Finished {
                completed: true,
                hex,
            }
        })
        .unwrap_or_else(|(psbt, errors)| {
            let errors: Vec<_> = errors.iter().map(|e| format!("{:?}", e)).collect();
            let encoded_psbt = base64::encode(serialize(&psbt));
            PSBTApi::NotFinished {
                completed: false,
                psbt: encoded_psbt,
                error: "Could not fully finalize psbt".into(),
                errors,
            }
        })
}

pub async fn read_key_from_file(
    file_name: &std::ffi::OsStr,
) -> Result<ExtendedPrivKey, Box<dyn Error>> {
    let buf = tokio::fs::read(file_name).await?;
    Ok(ExtendedPrivKey::decode(&buf)?)
}

pub fn sign(
    xpriv: ExtendedPrivKey,
    psbt: PartiallySignedTransaction,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let psbt = sign_psbt(&xpriv, psbt, &Secp256k1::new())?;
    let bytes = serialize(&psbt);
    Ok(bytes)
}
pub async fn show_pubkey(input: &std::ffi::OsStr) -> Result<(), Box<dyn Error>> {
    let buf = tokio::fs::read(input).await?;
    let xpriv = ExtendedPrivKey::decode(&buf)?;
    println!("{}", ExtendedPubKey::from_priv(&Secp256k1::new(), &xpriv));
    Ok(())
}
pub fn new_key(network: &str, out: &std::ffi::OsStr) -> Result<(), Box<dyn Error>> {
    let entropy: [u8; 32] = rand::thread_rng().gen();
    let xpriv = ExtendedPrivKey::new_master(Network::from_str(network)?, &entropy)?;
    std::fs::write(out, &xpriv.encode())?;
    println!("{}", ExtendedPubKey::from_priv(&Secp256k1::new(), &xpriv));
    Ok(())
}

fn input_err(s: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, s)
}
pub fn sign_psbt(
    xpriv: &ExtendedPrivKey,
    mut psbt: PartiallySignedTransaction,
    secp: &Secp256k1<All>,
) -> Result<PartiallySignedTransaction, std::io::Error> {
    let tx = psbt.clone().extract_tx();
    let utxos: Vec<TxOut> = psbt
        .inputs
        .iter()
        .map(|o| o.witness_utxo.clone())
        .collect::<Option<Vec<TxOut>>>()
        .ok_or_else(|| input_err("Could not find one of the UTXOs to be signed over"))?;
    let untweaked = xpriv.to_keypair(secp);
    let pk = XOnlyPublicKey::from_keypair(&untweaked);
    let mut sighash = bitcoin::util::sighash::SighashCache::new(&tx);
    let input_zero = &mut psbt.inputs[0];
    use bitcoin::schnorr::TapTweak;
    let tweaked = untweaked
        .tap_tweak(secp, input_zero.tap_merkle_root)
        .into_inner();
    let _tweaked_pk = tweaked.public_key();
    let hash_ty = bitcoin::util::sighash::SchnorrSighashType::All;
    let prevouts = &Prevouts::All(&utxos);
    if input_zero.tap_internal_key == Some(pk.0) {
        let sig = get_sig(&mut sighash, prevouts, hash_ty, secp, &tweaked, &None);
        input_zero.tap_key_sig = Some(sig);
    }

    let signers = compute_matching_keys(xpriv, secp, &input_zero.tap_key_origins);

    for (kp, vtlh) in signers {
        for tlh in vtlh {
            let sig = get_sig(
                &mut sighash,
                prevouts,
                hash_ty,
                secp,
                &untweaked,
                &Some((*tlh, DEFAULT_CODESEP)),
            );
            input_zero
                .tap_script_sigs
                .insert((kp.x_only_public_key().0, *tlh), sig);
        }
    }
    Ok(psbt)
}

/// Compute keypairs for all matching fingerprints
fn compute_matching_keys<'a>(
    xpriv: &'a ExtendedPrivKey,
    secp: &'a Secp256k1<All>,
    input_zero: &'a BTreeMap<XOnlyPublicKey, (Vec<TapLeafHash>, KeySource)>,
) -> impl Iterator<Item = (KeyPair, &'a Vec<TapLeafHash>)> + 'a {
    let fingerprint = xpriv.fingerprint(secp);
    input_zero
        .iter()
        .filter(move |(_, (_, (f, _)))| *f == fingerprint)
        .filter_map(|(x, (vlth, (_, path)))| {
            let new_priv = xpriv.derive_priv(secp, path).ok()?.to_keypair(secp);
            if new_priv.public_key().x_only_public_key().0 == *x {
                Some((new_priv, vlth))
            } else {
                None
            }
        })
}

const DEFAULT_CODESEP: u32 = 0xffff_ffff;
fn get_sig(
    sighash: &mut bitcoin::util::sighash::SighashCache<&bitcoin::Transaction>,
    prevouts: &Prevouts<TxOut>,
    hash_ty: bitcoin::SchnorrSighashType,
    secp: &Secp256k1<All>,
    kp: &bitcoin::KeyPair,
    path: &Option<(TapLeafHash, u32)>,
) -> SchnorrSig {
    let annex = None;
    let sighash: TapSighashHash = sighash
        .taproot_signature_hash(0, prevouts, annex, *path, hash_ty)
        .expect("Signature hash cannot fail...");
    let msg = bitcoin::secp256k1::Message::from_slice(&sighash[..]).expect("Size must be correct.");
    let sig = secp.sign_schnorr_no_aux_rand(&msg, kp);
    SchnorrSig { sig, hash_ty }
}
