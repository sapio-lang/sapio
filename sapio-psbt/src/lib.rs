// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use bitcoin::secp256k1::rand::Rng;
use bitcoin::secp256k1::{rand, All};
use bitcoin::util::sighash::Prevouts;
use bitcoin::util::taproot::TapLeafHash;
use bitcoin::util::taproot::TapSighashHash;
use bitcoin::TxOut;
use bitcoin::XOnlyPublicKey;
use bitcoin::{Network, SchnorrSig};
use miniscript::psbt::PsbtExt;
use tokio::io::AsyncReadExt;

pub async fn finalize_psbt(psbt_str: Option<&str>) -> Result<serde_json::Value, Box<dyn Error>> {
    let psbt: PartiallySignedTransaction = PartiallySignedTransaction::consensus_decode(
        &base64::decode(&if let Some(psbt) = psbt_str {
            psbt.into()
        } else {
            let mut s = String::new();
            tokio::io::stdin().read_to_string(&mut s).await?;
            s
        })?[..],
    )?;
    let secp = Secp256k1::new();
    let js = psbt
        .finalize(&secp)
        .map(|tx| {
            let hex = bitcoin::consensus::encode::serialize_hex(&tx.extract_tx());
            serde_json::json!({
                "completed": true,
                "hex": hex
            })
        })
        .unwrap_or_else(|(psbt, errors)| {
            let errors: Vec<_> = errors.iter().map(|e| format!("{:?}", e)).collect();
            let encoded_psbt = base64::encode(serialize(&psbt));
            serde_json::json!(
                {
                     "completed": false,
                     "psbt": encoded_psbt,
                     "error": "Could not fully finalize psbt",
                     "errors": errors
                }
            )
        });
    Ok(js)
}

use std::error::Error;
use std::str::FromStr;

use bitcoin::consensus::{serialize, Decodable};
use bitcoin::util::bip32::ExtendedPubKey;
use bitcoin::{
    psbt::PartiallySignedTransaction, secp256k1::Secp256k1, util::bip32::ExtendedPrivKey,
};

pub async fn sign(
    input: &std::ffi::OsStr,
    psbt_str: Option<&str>,
    output: Option<&std::ffi::OsStr>,
) -> Result<(), Box<dyn Error>> {
    {
        let buf = tokio::fs::read(input).await?;
        let xpriv = ExtendedPrivKey::decode(&buf)?;
        let psbt: PartiallySignedTransaction = PartiallySignedTransaction::consensus_decode(
            &base64::decode(&if let Some(psbt) = psbt_str {
                psbt.into()
            } else {
                let mut s = String::new();
                tokio::io::stdin().read_to_string(&mut s).await?;
                s
            })?[..],
        )?;
        let psbt = sign_psbt(&xpriv, psbt, &Secp256k1::new())?;
        let bytes = serialize(&psbt);
        if let Some(file_out) = output {
            std::fs::write(file_out, &base64::encode(bytes))?;
        } else {
            println!("{}", base64::encode(bytes));
        }
    };
    Ok(())
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
    let mut sighash = bitcoin::util::sighash::SighashCache::new(&tx);
    let input_zero = &mut b.inputs[0];
    use bitcoin::schnorr::TapTweak;
    let tweaked = untweaked
        .tap_tweak(secp, input_zero.tap_merkle_root)
        .into_inner();
    let _tweaked_pk = tweaked.public_key();
    let hash_ty = bitcoin::util::sighash::SchnorrSighashType::All;
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
    if input_zero.tap_internal_key == Some(pk.0) {
        let sig = get_sig(None, &tweaked);
        input_zero.tap_key_sig = Some(sig);
    }
    for tlh in input_zero
        .tap_scripts
        .values()
        .map(|(script, ver)| TapLeafHash::from_script(script, *ver))
    {
        let sig = get_sig(Some((tlh, 0xffffffff)), &untweaked);
        input_zero.tap_script_sigs.insert((pk.0.clone(), tlh), sig);
    }
    Ok(b)
}
