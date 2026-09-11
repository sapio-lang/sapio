// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use bitcoin::bip32::{Fingerprint, KeySource, Xpub};
use bitcoin::key::TapTweak;
use bitcoin::secp256k1::rand::Rng;
use bitcoin::secp256k1::Keypair;
use bitcoin::secp256k1::{rand, Signing, Verification};
use bitcoin::sighash::Prevouts;
use bitcoin::sighash::TapSighash;
use bitcoin::taproot::TapLeafHash;
use bitcoin::Network;
use bitcoin::TxOut;
use bitcoin::XOnlyPublicKey;
use bitcoin::{bip32::Xpriv, psbt::Psbt, secp256k1::Secp256k1};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Display;
pub mod annex;
pub mod external_api;
pub mod finalize;

pub struct SigningKey(pub Vec<Xpriv>);

impl SigningKey {
    pub fn read_key_from_buf(buf: &[u8]) -> Result<Self, bitcoin::bip32::Error> {
        Xpriv::decode(buf).map(|k| SigningKey(vec![k]))
    }
    pub fn new_key(network: Network) -> Result<Self, bitcoin::bip32::Error> {
        let seed: [u8; 32] = rand::thread_rng().gen();
        let xpriv = Xpriv::new_master(network, &seed)?;
        Ok(SigningKey(vec![xpriv]))
    }
    pub fn merge(&mut self, other: SigningKey) -> &mut SigningKey {
        self.0.extend(other.0);
        self
    }
    pub fn pubkey<C: Signing>(&self, secp: &Secp256k1<C>) -> Vec<Xpub> {
        self.0.iter().map(|s| Xpub::from_priv(secp, s)).collect()
    }
    pub fn sign(
        &self,
        mut psbt: Psbt,
        hash_ty: bitcoin::TapSighashType,
    ) -> Result<Vec<u8>, PSBTSigningError> {
        self.sign_psbt_mut(&mut psbt, &Secp256k1::new(), hash_ty)?;
        let bytes = psbt.serialize();
        Ok(bytes)
    }
    pub fn sign_psbt<C: Signing + Verification>(
        &self,
        mut psbt: Psbt,
        secp: &Secp256k1<C>,
        hash_ty: bitcoin::TapSighashType,
    ) -> Result<Psbt, (Psbt, PSBTSigningError)> {
        match self.sign_psbt_mut(&mut psbt, secp, hash_ty) {
            Ok(()) => Ok(psbt),
            Err(e) => Err((psbt, e)),
        }
    }
    pub fn sign_psbt_mut<C: Signing + Verification>(
        &self,
        psbt: &mut Psbt,
        secp: &Secp256k1<C>,
        hash_ty: bitcoin::TapSighashType,
    ) -> Result<(), PSBTSigningError> {
        validate_psbt(psbt)?;
        let l = psbt.inputs.len();
        for idx in 0..l {
            self.sign_validated_psbt_input_mut(psbt, secp, idx, hash_ty)?;
        }
        Ok(())
    }
    pub fn sign_psbt_input<C: Signing + Verification>(
        &self,
        mut psbt: Psbt,
        secp: &Secp256k1<C>,
        idx: usize,
        hash_ty: bitcoin::TapSighashType,
    ) -> Result<Psbt, (Psbt, PSBTSigningError)> {
        match self.sign_psbt_input_mut(&mut psbt, secp, idx, hash_ty) {
            Ok(()) => Ok(psbt),
            Err(e) => Err((psbt, e)),
        }
    }
    pub fn sign_psbt_input_mut<C: Signing + Verification>(
        &self,
        psbt: &mut Psbt,
        secp: &Secp256k1<C>,
        idx: usize,
        hash_ty: bitcoin::TapSighashType,
    ) -> Result<(), PSBTSigningError> {
        validate_psbt(psbt)?;
        self.sign_validated_psbt_input_mut(psbt, secp, idx, hash_ty)
    }

    fn sign_validated_psbt_input_mut<C: Signing + Verification>(
        &self,
        psbt: &mut Psbt,
        secp: &Secp256k1<C>,
        idx: usize,
        hash_ty: bitcoin::TapSighashType,
    ) -> Result<(), PSBTSigningError> {
        let tx = &psbt.unsigned_tx;
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
        let mut sighash = bitcoin::sighash::SighashCache::new(tx);
        let input = &mut psbt
            .inputs
            .get_mut(idx)
            .ok_or(PSBTSigningError::NoInputAtIndex(idx))?;
        if input.final_script_sig.is_some() || input.final_script_witness.is_some() {
            return Ok(());
        }
        if annex::get(input)
            .map_err(|error| PSBTValidationError::InvalidAnnex { index: idx, error })?
            .is_some()
            && !utxos[idx].script_pubkey.is_p2tr()
        {
            return Err(PSBTSigningError::AnnexRequiresTaproot(idx));
        }
        let prevouts = &Prevouts::All(&utxos);
        let fingerprints_map = self.compute_fingerprint_map(secp);
        self.sign_taproot_top_key(
            secp,
            input,
            idx,
            &mut sighash,
            prevouts,
            hash_ty,
            &fingerprints_map,
        )?;
        self.sign_all_tapleaf_branches(
            secp,
            input,
            idx,
            &mut sighash,
            prevouts,
            hash_ty,
            &fingerprints_map,
        )?;
        Ok(())
    }

    fn sign_all_tapleaf_branches<C: Signing + Verification>(
        &self,
        secp: &Secp256k1<C>,
        input: &mut bitcoin::psbt::Input,
        input_index: usize,
        sighash: &mut bitcoin::sighash::SighashCache<&bitcoin::Transaction>,
        prevouts: &Prevouts<TxOut>,
        hash_ty: bitcoin::TapSighashType,
        fingerprints_map: &Vec<(Fingerprint, &Xpriv)>,
    ) -> Result<(), PSBTSigningError> {
        let signers = self.compute_matching_keys(secp, &input.tap_key_origins, fingerprints_map);
        for (kp, vtlh) in signers {
            for tlh in vtlh {
                let sig = get_sig(
                    sighash,
                    input_index,
                    prevouts,
                    hash_ty,
                    secp,
                    &kp,
                    &Some((*tlh, DEFAULT_CODESEP)),
                    annex::get(input).map_err(|error| PSBTValidationError::InvalidAnnex {
                        index: input_index,
                        error,
                    })?,
                )?;
                input
                    .tap_script_sigs
                    .insert((kp.x_only_public_key().0, *tlh), sig);
            }
        }
        Ok(())
    }

    fn sign_taproot_top_key<C: Signing + Verification>(
        &self,
        secp: &Secp256k1<C>,
        input: &mut bitcoin::psbt::Input,
        input_index: usize,
        sighash: &mut bitcoin::sighash::SighashCache<&bitcoin::Transaction>,
        prevouts: &Prevouts<TxOut>,
        hash_ty: bitcoin::TapSighashType,
        fingerprints_map: &Vec<(Fingerprint, &Xpriv)>,
    ) -> Result<(), PSBTSigningError> {
        // first attempt to use derivations from the key source map
        let Some(key) = input.tap_internal_key else {
            return Ok(());
        };
        let Some(untweaked) = self.find_internal_keypair(input, key, fingerprints_map, secp) else {
            return Ok(());
        };
        let tweaked = untweaked
            .tap_tweak(secp, input.tap_merkle_root)
            .to_keypair();
        input.tap_key_sig = Some(get_sig(
            sighash,
            input_index,
            prevouts,
            hash_ty,
            secp,
            &tweaked,
            &None,
            annex::get(input).map_err(|error| PSBTValidationError::InvalidAnnex {
                index: input_index,
                error,
            })?,
        )?);
        Ok(())
    }

    fn find_internal_keypair<C: Signing>(
        &self,
        input: &mut bitcoin::psbt::Input,
        input_key: XOnlyPublicKey,
        fingerprints_map: &Vec<(Fingerprint, &Xpriv)>,
        secp: &Secp256k1<C>,
    ) -> Option<Keypair> {
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
        fingerprints_map: &'a Vec<(Fingerprint, &'a Xpriv)>,
    ) -> impl Iterator<Item = (Keypair, &'a Vec<TapLeafHash>)> + 'a {
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
    ) -> Vec<(Fingerprint, &'a Xpriv)> {
        let fingerprint = self.0.iter().map(|k| (k.fingerprint(secp), k));
        let mut keyarr: Vec<(Fingerprint, &Xpriv)> = fingerprint.collect();
        keyarr.sort_by_key(|k| k.0);
        keyarr
    }
}

#[derive(Debug, Clone)]
pub enum PSBTSigningError {
    InvalidPSBT(PSBTValidationError),
    NoUTXOAtIndex(usize),
    NoInputAtIndex(usize),
    AnnexRequiresTaproot(usize),
    Sighash(bitcoin::sighash::TaprootError),
}

impl Display for PSBTSigningError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPSBT(error) => Display::fmt(error, f),
            Self::NoUTXOAtIndex(index) => write!(f, "missing witness UTXO for input {index}"),
            Self::NoInputAtIndex(index) => write!(f, "no input at index {index}"),
            Self::AnnexRequiresTaproot(index) => write!(
                f,
                "annex requires a Taproot previous output at input {index}"
            ),
            Self::Sighash(error) => write!(f, "cannot compute signature hash: {error}"),
        }
    }
}
impl Error for PSBTSigningError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidPSBT(error) => Some(error),
            Self::Sighash(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PSBTValidationError> for PSBTSigningError {
    fn from(error: PSBTValidationError) -> Self {
        Self::InvalidPSBT(error)
    }
}

/// A PSBT whose public fields violate the signing and finalization boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PSBTValidationError {
    NoInputs,
    InputMapCount {
        transaction: usize,
        maps: usize,
    },
    OutputMapCount {
        transaction: usize,
        maps: usize,
    },
    UnsignedTxHasScriptSig(usize),
    UnsignedTxHasWitness(usize),
    InvalidAnnex {
        index: usize,
        error: annex::AnnexError,
    },
}

impl Display for PSBTValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoInputs => write!(f, "PSBT unsigned transaction has no inputs"),
            Self::InputMapCount { transaction, maps } => write!(
                f,
                "PSBT has {maps} input maps for {transaction} transaction inputs"
            ),
            Self::OutputMapCount { transaction, maps } => write!(
                f,
                "PSBT has {maps} output maps for {transaction} transaction outputs"
            ),
            Self::UnsignedTxHasScriptSig(index) => {
                write!(f, "PSBT unsigned transaction input {index} has a scriptSig")
            }
            Self::UnsignedTxHasWitness(index) => {
                write!(f, "PSBT unsigned transaction input {index} has a witness")
            }
            Self::InvalidAnnex { index, error } => {
                write!(f, "invalid annex for input {index}: {error}")
            }
        }
    }
}

impl Error for PSBTValidationError {}

/// Validate transaction and map structure before extracting or signing a PSBT.
///
/// Public fields can bypass checks performed during deserialization. This
/// checks input presence, map counts and unsigned transaction fields; it does
/// not authenticate previous outputs or verify signatures.
pub fn validate_psbt(psbt: &Psbt) -> Result<(), PSBTValidationError> {
    if psbt.unsigned_tx.input.is_empty() {
        return Err(PSBTValidationError::NoInputs);
    }
    if psbt.inputs.len() != psbt.unsigned_tx.input.len() {
        return Err(PSBTValidationError::InputMapCount {
            transaction: psbt.unsigned_tx.input.len(),
            maps: psbt.inputs.len(),
        });
    }
    if psbt.outputs.len() != psbt.unsigned_tx.output.len() {
        return Err(PSBTValidationError::OutputMapCount {
            transaction: psbt.unsigned_tx.output.len(),
            maps: psbt.outputs.len(),
        });
    }
    for (index, input) in psbt.unsigned_tx.input.iter().enumerate() {
        if !input.script_sig.is_empty() {
            return Err(PSBTValidationError::UnsignedTxHasScriptSig(index));
        }
        if !input.witness.is_empty() {
            return Err(PSBTValidationError::UnsignedTxHasWitness(index));
        }
        annex::get(&psbt.inputs[index])
            .map_err(|error| PSBTValidationError::InvalidAnnex { index, error })?;
    }
    Ok(())
}

const DEFAULT_CODESEP: u32 = 0xffff_ffff;
fn get_sig<C: Signing>(
    sighash: &mut bitcoin::sighash::SighashCache<&bitcoin::Transaction>,
    input_index: usize,
    prevouts: &Prevouts<TxOut>,
    hash_ty: bitcoin::TapSighashType,
    secp: &Secp256k1<C>,
    kp: &Keypair,
    path: &Option<(TapLeafHash, u32)>,
    annex: Option<&[u8]>,
) -> Result<bitcoin::taproot::Signature, PSBTSigningError> {
    let annex = annex
        .map(|bytes| {
            annex::validate(bytes).map_err(|error| PSBTValidationError::InvalidAnnex {
                index: input_index,
                error,
            })?;
            Ok::<_, PSBTValidationError>(
                bitcoin::sighash::Annex::new(bytes).expect("validated annex"),
            )
        })
        .transpose()?;
    let sighash: TapSighash = sighash
        .taproot_signature_hash(input_index, prevouts, annex, *path, hash_ty)
        .map_err(PSBTSigningError::Sighash)?;
    let msg = bitcoin::secp256k1::Message::from_digest_slice(&sighash[..])
        .expect("Taproot signature hashes are 32 bytes");
    let sig = secp.sign_schnorr_no_aux_rand(&msg, kp);
    Ok(bitcoin::taproot::Signature {
        signature: sig,
        sighash_type: hash_ty,
    })
}
