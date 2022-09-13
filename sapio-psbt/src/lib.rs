// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use bitcoin::consensus::serialize;
use bitcoin::schnorr::TapTweak;
use bitcoin::secp256k1::rand::Rng;
use bitcoin::secp256k1::{rand, Signing, Verification};
use bitcoin::util::bip32::{ExtendedPubKey, Fingerprint, KeySource};
use bitcoin::util::sighash::Prevouts;
use bitcoin::util::taproot::TapLeafHash;
use bitcoin::util::taproot::TapSighashHash;
use bitcoin::XOnlyPublicKey;
use bitcoin::{
    psbt::PartiallySignedTransaction, secp256k1::Secp256k1, util::bip32::ExtendedPrivKey,
};
use bitcoin::{KeyPair, TxOut};
use bitcoin::{Network, SchnorrSig};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Display;
pub mod external_api;

pub struct SigningKey(pub Vec<ExtendedPrivKey>);

impl SigningKey {
    pub fn read_key_from_buf(buf: &[u8]) -> Result<Self, bitcoin::util::bip32::Error> {
        ExtendedPrivKey::decode(buf).map(|k| SigningKey(vec![k]))
    }
    pub fn new_key(network: Network) -> Result<Self, bitcoin::util::bip32::Error> {
        let seed: [u8; 32] = rand::thread_rng().gen();
        let xpriv = ExtendedPrivKey::new_master(network, &seed)?;
        Ok(SigningKey(vec![xpriv]))
    }
    pub fn merge(&mut self, other: SigningKey) -> &mut SigningKey {
        self.0.extend(other.0);
        self
    }
    pub fn pubkey<C: Signing>(&self, secp: &Secp256k1<C>) -> Vec<ExtendedPubKey> {
        self.0
            .iter()
            .map(|s| ExtendedPubKey::from_priv(secp, s))
            .collect()
    }
    pub fn sign(
        &self,
        mut psbt: PartiallySignedTransaction,
        hash_ty: bitcoin::SchnorrSighashType,
    ) -> Result<Vec<u8>, PSBTSigningError> {
        self.sign_psbt_mut(&mut psbt, &Secp256k1::new(), hash_ty)?;
        let bytes = serialize(&psbt);
        Ok(bytes)
    }
    pub fn sign_psbt<C: Signing + Verification>(
        &self,
        mut psbt: PartiallySignedTransaction,
        secp: &Secp256k1<C>,
        hash_ty: bitcoin::SchnorrSighashType,
    ) -> Result<PartiallySignedTransaction, (PartiallySignedTransaction, PSBTSigningError)> {
        match self.sign_psbt_mut(&mut psbt, secp, hash_ty) {
            Ok(()) => Ok(psbt),
            Err(e) => Err((psbt, e)),
        }
    }
    pub fn sign_psbt_mut<C: Signing + Verification>(
        &self,
        psbt: &mut PartiallySignedTransaction,
        secp: &Secp256k1<C>,
        hash_ty: bitcoin::SchnorrSighashType,
    ) -> Result<(), PSBTSigningError> {
        let l = psbt.inputs.len();
        for idx in 0..l {
            self.sign_psbt_input_mut(psbt, secp, idx, hash_ty)?;
        }
        Ok(())
    }
    pub fn sign_psbt_input<C: Signing + Verification>(
        &self,
        mut psbt: PartiallySignedTransaction,
        secp: &Secp256k1<C>,
        idx: usize,
        hash_ty: bitcoin::SchnorrSighashType,
    ) -> Result<PartiallySignedTransaction, (PartiallySignedTransaction, PSBTSigningError)> {
        match self.sign_psbt_input_mut(&mut psbt, secp, idx, hash_ty) {
            Ok(()) => Ok(psbt),
            Err(e) => Err((psbt, e)),
        }
    }
    pub fn sign_psbt_input_mut<C: Signing + Verification>(
        &self,
        psbt: &mut PartiallySignedTransaction,
        secp: &Secp256k1<C>,
        idx: usize,
        hash_ty: bitcoin::SchnorrSighashType,
    ) -> Result<(), PSBTSigningError> {
        let tx = psbt.clone().extract_tx();
        let utxos: Vec<TxOut> = psbt
            .inputs
            .iter()
            .enumerate()
            .map(|(i, o)| {
                if let Some(ref utxo) = o.witness_utxo {
                    Ok(utxo.clone())
                } else {
                    Err(i)
                }
            })
            .collect::<Result<Vec<TxOut>, usize>>()
            .map_err(PSBTSigningError::NoUTXOAtIndex)?;
        let mut sighash = bitcoin::util::sighash::SighashCache::new(&tx);
        let input = &mut psbt
            .inputs
            .get_mut(idx)
            .ok_or(PSBTSigningError::NoInputAtIndex(idx))?;
        let prevouts = &Prevouts::All(&utxos);
        let fingerprints_map = self.compute_fingerprint_map(secp);
        self.sign_taproot_top_key(
            secp,
            input,
            &mut sighash,
            prevouts,
            hash_ty,
            &fingerprints_map,
        );
        self.sign_all_tapleaf_branches(
            secp,
            input,
            &mut sighash,
            prevouts,
            hash_ty,
            &fingerprints_map,
        );
        Ok(())
    }

    fn sign_all_tapleaf_branches<C: Signing + Verification>(
        &self,
        secp: &Secp256k1<C>,
        input: &mut bitcoin::psbt::Input,
        sighash: &mut bitcoin::util::sighash::SighashCache<&bitcoin::Transaction>,
        prevouts: &Prevouts<TxOut>,
        hash_ty: bitcoin::SchnorrSighashType,
        fingerprints_map: &Vec<(Fingerprint, &ExtendedPrivKey)>,
    ) {
        let signers = self.compute_matching_keys(secp, &input.tap_key_origins, fingerprints_map);
        for (kp, vtlh) in signers {
            for tlh in vtlh {
                let sig = get_sig(
                    sighash,
                    prevouts,
                    hash_ty,
                    secp,
                    &kp,
                    &Some((*tlh, DEFAULT_CODESEP)),
                );
                input
                    .tap_script_sigs
                    .insert((kp.x_only_public_key().0, *tlh), sig);
            }
        }
    }

    fn sign_taproot_top_key<C: Signing + Verification>(
        &self,
        secp: &Secp256k1<C>,
        input: &mut bitcoin::psbt::Input,
        sighash: &mut bitcoin::util::sighash::SighashCache<&bitcoin::Transaction>,
        prevouts: &Prevouts<TxOut>,
        hash_ty: bitcoin::SchnorrSighashType,
        fingerprints_map: &Vec<(Fingerprint, &ExtendedPrivKey)>,
    ) -> Option<()> {
        // first attempt to use derivations from the key source map
        let key = input.tap_internal_key?;
        let untweaked = self.find_internal_keypair(input, key, fingerprints_map, secp)?;
        let tweaked = untweaked
            .tap_tweak(secp, input.tap_merkle_root)
            .into_inner();
        input.tap_key_sig = Some(get_sig(sighash, prevouts, hash_ty, secp, &tweaked, &None));
        Some(())
    }

    fn find_internal_keypair<C: Signing>(
        &self,
        input: &mut bitcoin::psbt::Input,
        input_key: XOnlyPublicKey,
        fingerprints_map: &Vec<(Fingerprint, &ExtendedPrivKey)>,
        secp: &Secp256k1<C>,
    ) -> Option<KeyPair> {
        // Assume that the key is an exact, non derived, match for a key we know already
        for kp in self.0.iter() {
            let untweaked = kp.to_keypair(secp);
            let pk = XOnlyPublicKey::from_keypair(&untweaked);
            if input_key == pk.0 {
                return Some(untweaked);
            }
        }
        // Otherwise, try to derive a key
        let (_, (f, path)) = input.tap_key_origins.get(&input_key)?;
        let idx = fingerprints_map.partition_point(|(x, _)| *x < *f);
        for (_, key) in fingerprints_map.iter().skip(idx).take_while(|k| k.0 == *f) {
            if let Ok(sk) = key.derive_priv(secp, path) {
                let untweaked = sk.to_keypair(secp);
                let pk = untweaked.public_key().x_only_public_key().0;
                if pk == input_key {
                    return Some(untweaked);
                }
            }
        }
        None
    }

    /// Compute keypairs for all matching fingerprints
    fn compute_matching_keys<'a, C: Signing>(
        &'a self,
        secp: &'a Secp256k1<C>,
        input: &'a BTreeMap<XOnlyPublicKey, (Vec<TapLeafHash>, KeySource)>,
        fingerprints_map: &'a Vec<(Fingerprint, &'a ExtendedPrivKey)>,
    ) -> impl Iterator<Item = (KeyPair, &'a Vec<TapLeafHash>)> + 'a {
        // TODO: Cache this on type creation?
        input.iter().filter_map(move |(x, (vlth, (f, path)))| {
            let idx = fingerprints_map.partition_point(|(x, _)| *x < *f);
            for (_, key) in fingerprints_map
                .iter()
                .skip(idx)
                .take_while(|(x, _)| *x == *f)
            {
                match key.derive_priv(secp, path).map(|k| k.to_keypair(secp)) {
                    Ok(kp) => {
                        if kp.public_key().x_only_public_key().0 == *x {
                            return Some((kp, vlth));
                        } else {
                            return None;
                        }
                    }
                    Err(_) => continue,
                }
            }
            None
        })
    }

    /// Computes a map of all fingerprints
    // TODO: consider more memory efficient representations
    fn compute_fingerprint_map<'a, C: Signing>(
        &'a self,
        secp: &Secp256k1<C>,
    ) -> Vec<(Fingerprint, &'a ExtendedPrivKey)> {
        let fingerprint = self.0.iter().map(|k| (k.fingerprint(secp), k));
        let mut keyarr: Vec<(Fingerprint, &ExtendedPrivKey)> = fingerprint.collect();
        keyarr.sort_by_key(|k| k.0);
        keyarr
    }
}

#[derive(Debug, Clone)]
pub enum PSBTSigningError {
    NoUTXOAtIndex(usize),
    NoInputAtIndex(usize),
}

impl Display for PSBTSigningError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}
impl Error for PSBTSigningError {}

const DEFAULT_CODESEP: u32 = 0xffff_ffff;
fn get_sig<C: Signing>(
    sighash: &mut bitcoin::util::sighash::SighashCache<&bitcoin::Transaction>,
    prevouts: &Prevouts<TxOut>,
    hash_ty: bitcoin::SchnorrSighashType,
    secp: &Secp256k1<C>,
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
