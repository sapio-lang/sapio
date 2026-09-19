//! Fill a chosen satisfaction without choosing another spending branch.
//!
//! Recipes and native signature permissions must come from an authenticated
//! policy selection. This module verifies their completed transaction, but
//! does not decide whether a caller is authorized to choose that policy.

use crate::{annex, finalize, validate_psbt, PSBTSigningError, PSBTValidationError, SigningKey};
use bitcoin::blockdata::{opcodes::all::OP_CODESEPARATOR, script::Instruction};
use bitcoin::hashes::{hash160, ripemd160, sha256, sha256d, Hash};
use bitcoin::key::TapTweak;
use bitcoin::psbt::{Input, Psbt};
use bitcoin::script::{Builder, PushBytesBuf};
use bitcoin::secp256k1::{Message, Secp256k1, SecretKey, Signing, Verification};
use bitcoin::sighash::{Annex, Prevouts, SighashCache};
use bitcoin::taproot::{LeafVersion, TapLeafHash};
use bitcoin::{PublicKey, Script, ScriptBuf, TxOut, Witness, XOnlyPublicKey};
use miniscript::psbt::PsbtExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt};

/// The digest of a 32-byte Miniscript preimage.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub enum HashRequirement {
    /// SHA256.
    Sha256(sha256::Hash),
    /// Double SHA256.
    Hash256(sha256d::Hash),
    /// RIPEMD160.
    Ripemd160(ripemd160::Hash),
    /// RIPEMD160 of SHA256.
    Hash160(hash160::Hash),
}

/// One exact position in a selected spending stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StackElement {
    /// Public bytes, including selectors, scripts, proofs, or annex bytes.
    Literal(Vec<u8>),
    /// An ECDSA signature from this input's partial signature map.
    EcdsaSignature(PublicKey),
    /// A Schnorr signature; no leaf denotes the tweaked key path.
    SchnorrSignature {
        /// Untweaked signing key.
        key: XOnlyPublicKey,
        /// Exact script leaf, or the key path.
        leaf: Option<TapLeafHash>,
    },
    /// A preimage whose digest and length must both match.
    Preimage(HashRequirement),
}

/// Exact stack packaging selected by a policy planner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SatisfactionRecipe {
    /// Elements encoded as minimal scriptSig pushes, in execution order.
    pub script_sig: Vec<StackElement>,
    /// Witness elements, including script/control block/annex when applicable.
    pub witness: Vec<StackElement>,
}

/// Explicit permission for an ordinary native signature at one input.
///
/// Program/emulator signature slots must be excluded: possessing a private
/// key is not authorization to bypass its evaluator.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SignatureSlot {
    /// A native ECDSA key.
    Ecdsa(PublicKey),
    /// A native Schnorr key at exactly one spending path.
    Schnorr {
        /// Untweaked signing key.
        key: XOnlyPublicKey,
        /// Exact script leaf, or the key path.
        leaf: Option<TapLeafHash>,
    },
}

/// A selected signing or completion failure.
#[derive(Debug)]
pub enum SelectedError {
    /// Malformed public PSBT fields.
    InvalidPSBT(PSBTValidationError),
    /// Conflicting or malformed supplied funding evidence.
    Funding(sapio_base::psbt::FundingError),
    /// Invalid metadata or selected stack at an input.
    Input { index: usize, reason: &'static str },
    /// A selected signature or preimage has not been supplied.
    MissingAsset { index: usize, element: StackElement },
    /// A Schnorr signing failure.
    Signing(PSBTSigningError),
    /// An ECDSA signature hash could not be computed.
    Sighash(miniscript::psbt::SighashError),
    /// The completed transaction failed script verification.
    Finalization(Vec<finalize::FinalizationError>),
}

impl fmt::Display for SelectedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPSBT(error) => error.fmt(f),
            Self::Funding(error) => error.fmt(f),
            Self::Input { index, reason } => write!(f, "input {index}: {reason}"),
            Self::MissingAsset { index, element } => {
                write!(f, "input {index}: missing selected asset {element:?}")
            }
            Self::Signing(error) => error.fmt(f),
            Self::Sighash(error) => error.fmt(f),
            Self::Finalization(errors) => {
                write!(f, "selected satisfaction failed verification")?;
                for error in errors {
                    write!(f, "; {error}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for SelectedError {}

fn invalid(index: usize, reason: &'static str) -> SelectedError {
    SelectedError::Input { index, reason }
}

fn prevouts(psbt: &Psbt) -> Result<Vec<TxOut>, SelectedError> {
    validate_psbt(psbt).map_err(SelectedError::InvalidPSBT)?;
    sapio_base::psbt::previous_outputs(psbt)
        .map_err(SelectedError::Funding)?
        .into_iter()
        .enumerate()
        .map(|(index, output)| {
            output
                .cloned()
                .ok_or_else(|| invalid(index, "missing previous output"))
        })
        .collect()
}

impl SigningKey {
    /// Sign only explicitly permitted native slots at one input.
    ///
    /// Unknown keys are skipped and the number of signatures written is
    /// returned. The input is unchanged on error. Sighash flags come from the
    /// PSBT, defaulting to ALL for ECDSA and DEFAULT for Schnorr.
    pub fn sign_selected_input_mut<C: Signing + Verification>(
        &self,
        psbt: &mut Psbt,
        secp: &Secp256k1<C>,
        index: usize,
        native_slots: &[SignatureSlot],
    ) -> Result<usize, SelectedError> {
        let utxos = prevouts(psbt)?;
        let mut input = psbt
            .inputs
            .get(index)
            .ok_or_else(|| invalid(index, "input index out of bounds"))?
            .clone();
        if input.final_script_sig.is_some() || input.final_script_witness.is_some() {
            return Ok(0);
        }
        let script = &utxos[index].script_pubkey;
        let annex = annex::get(&input)
            .map_err(|error| {
                SelectedError::InvalidPSBT(PSBTValidationError::InvalidAnnex { index, error })
            })?
            .map(<[u8]>::to_vec);
        if annex.is_some() && !script.is_p2tr() {
            return Err(invalid(
                index,
                "annex requires a native Taproot previous output",
            ));
        }
        let fingerprints = self.compute_fingerprint_map(secp);
        let mut cache = SighashCache::new(&psbt.unsigned_tx);
        let mut signed = 0;
        for slot in native_slots {
            match *slot {
                SignatureSlot::Schnorr { key, leaf } => {
                    validate_taproot_slot(&input, script, secp, index, key, leaf)?;
                    let existing = leaf.map_or(input.tap_key_sig, |leaf| {
                        input.tap_script_sigs.get(&(key, leaf)).copied()
                    });
                    if let Some(signature) = existing {
                        if input.sighash_type.is_some_and(|required| {
                            required.to_u32() != signature.sighash_type as u32
                        }) {
                            return Err(invalid(
                                index,
                                "existing selected signature has a different sighash type",
                            ));
                        }
                        let digest = cache
                            .taproot_signature_hash(
                                index,
                                &Prevouts::All(&utxos),
                                annex
                                    .as_deref()
                                    .map(|bytes| Annex::new(bytes).expect("validated annex")),
                                leaf.map(|leaf| (leaf, crate::DEFAULT_CODESEP)),
                                signature.sighash_type,
                            )
                            .map_err(|error| {
                                SelectedError::Signing(PSBTSigningError::Sighash(error))
                            })?;
                        let verify_key = if leaf.is_none() {
                            key.tap_tweak(secp, input.tap_merkle_root)
                                .0
                                .to_x_only_public_key()
                        } else {
                            key
                        };
                        secp.verify_schnorr(
                            &signature.signature,
                            &Message::from_digest(digest.to_byte_array()),
                            &verify_key,
                        )
                        .map_err(|_| {
                            invalid(index, "existing selected Schnorr signature is invalid")
                        })?;
                        continue;
                    }
                    let hash_type = input
                        .sighash_type
                        .map(|ty| ty.taproot_hash_ty())
                        .unwrap_or(Ok(bitcoin::TapSighashType::Default))
                        .map_err(|_| invalid(index, "invalid Taproot sighash type"))?;
                    let Some(mut keypair) =
                        self.find_internal_keypair(&mut input, key, &fingerprints, secp)
                    else {
                        continue;
                    };
                    if leaf.is_none() {
                        keypair = keypair.tap_tweak(secp, input.tap_merkle_root).to_keypair();
                    }
                    let signature = crate::get_sig(
                        &mut cache,
                        index,
                        &Prevouts::All(&utxos),
                        hash_type,
                        secp,
                        &keypair,
                        &leaf.map(|leaf| (leaf, crate::DEFAULT_CODESEP)),
                        annex.as_deref(),
                    )
                    .map_err(SelectedError::Signing)?;
                    if let Some(leaf) = leaf {
                        input.tap_script_sigs.insert((key, leaf), signature);
                    } else {
                        input.tap_key_sig = Some(signature);
                    }
                    signed += 1;
                }
                SignatureSlot::Ecdsa(key) => {
                    validate_ecdsa_script(&input, script, index)?;
                    if let Some(signature) = input.partial_sigs.get(&key) {
                        if input.sighash_type.is_some_and(|required| {
                            required.to_u32() != signature.sighash_type as u32
                        }) {
                            return Err(invalid(
                                index,
                                "existing selected signature has a different sighash type",
                            ));
                        }
                        // PsbtExt hashes the declared flag. Supply the existing
                        // signature's flag when the PSBT left it unspecified.
                        let mut view = psbt.clone();
                        view.inputs[index].sighash_type = Some(signature.sighash_type.into());
                        let message = view
                            .sighash_msg(index, &mut cache, None)
                            .map_err(SelectedError::Sighash)?
                            .to_secp_msg();
                        secp.verify_ecdsa(&message, &signature.signature, &key.inner)
                            .map_err(|_| {
                                invalid(index, "existing selected ECDSA signature is invalid")
                            })?;
                        continue;
                    }
                    let hash_type = input
                        .sighash_type
                        .map(|ty| ty.ecdsa_hash_ty())
                        .unwrap_or(Ok(bitcoin::EcdsaSighashType::All))
                        .map_err(|_| invalid(index, "invalid ECDSA sighash type"))?;
                    // Do not sign the historical SIGHASH_SINGLE constant-one
                    // result, even for legacy inputs where consensus permits it.
                    if matches!(
                        hash_type,
                        bitcoin::EcdsaSighashType::Single
                            | bitcoin::EcdsaSighashType::SinglePlusAnyoneCanPay
                    ) && index >= psbt.unsigned_tx.output.len()
                    {
                        return Err(invalid(index, "SIGHASH_SINGLE has no corresponding output"));
                    }
                    let Some(secret) = self.selected_ecdsa_key(&input, key, secp) else {
                        continue;
                    };
                    let message = psbt
                        .sighash_msg(index, &mut cache, None)
                        .map_err(SelectedError::Sighash)?
                        .to_secp_msg();
                    input.partial_sigs.insert(
                        key,
                        bitcoin::ecdsa::Signature {
                            signature: secp.sign_ecdsa(&message, &secret),
                            sighash_type: hash_type,
                        },
                    );
                    signed += 1;
                }
            }
        }
        psbt.inputs[index] = input;
        Ok(signed)
    }

    fn selected_ecdsa_key<C: Signing>(
        &self,
        input: &Input,
        key: PublicKey,
        secp: &Secp256k1<C>,
    ) -> Option<SecretKey> {
        for root in &self.0 {
            if root.to_keypair(secp).public_key() == key.inner {
                return Some(root.private_key);
            }
            if let Some((fingerprint, path)) = input.bip32_derivation.get(&key.inner) {
                if root.fingerprint(secp) == *fingerprint {
                    if let Ok(derived) = root.derive_priv(secp, path) {
                        if derived.to_keypair(secp).public_key() == key.inner {
                            return Some(derived.private_key);
                        }
                    }
                }
            }
        }
        None
    }
}

fn validate_taproot_slot<C: Verification>(
    input: &Input,
    script: &Script,
    secp: &Secp256k1<C>,
    index: usize,
    key: XOnlyPublicKey,
    leaf: Option<TapLeafHash>,
) -> Result<(), SelectedError> {
    if !script.is_p2tr() {
        return Err(invalid(
            index,
            "Schnorr slot requires a native Taproot previous output",
        ));
    }
    if let Some(leaf) = leaf {
        let output_key = XOnlyPublicKey::from_slice(&script.as_bytes()[2..])
            .map_err(|_| invalid(index, "invalid Taproot output key"))?;
        let valid = input
            .tap_scripts
            .iter()
            .any(|(control, (script, version))| {
                *version == LeafVersion::TapScript
                    && control.leaf_version == *version
                    && TapLeafHash::from_script(script, *version) == leaf
                    && control.verify_taproot_commitment(secp, output_key, script)
                    && supported_script(script)
            });
        if !valid {
            return Err(invalid(
                index,
                "selected Taproot leaf lacks a valid script proof or contains a code separator",
            ));
        }
    } else if input.tap_internal_key != Some(key)
        || ScriptBuf::new_p2tr(secp, key, input.tap_merkle_root).as_script() != script
    {
        return Err(invalid(
            index,
            "selected internal key and tweak do not match the previous output",
        ));
    }
    Ok(())
}

fn supported_script(script: &Script) -> bool {
    script.instructions().all(|instruction| match instruction {
        Ok(Instruction::Op(opcode)) => opcode != OP_CODESEPARATOR,
        Ok(_) => true,
        Err(_) => false,
    })
}

fn validate_ecdsa_script(
    input: &Input,
    output: &Script,
    index: usize,
) -> Result<(), SelectedError> {
    if output.is_p2tr() {
        return Err(invalid(index, "ECDSA slot cannot sign a Taproot input"));
    }
    let script = if output.is_p2sh() {
        let redeem = input
            .redeem_script
            .as_deref()
            .ok_or_else(|| invalid(index, "missing selected redeem script"))?;
        if redeem.to_p2sh().as_script() != output {
            return Err(invalid(
                index,
                "redeem script does not match the previous output",
            ));
        }
        redeem
    } else {
        output
    };
    let segwit = script.is_p2wpkh() || script.is_p2wsh();
    if !segwit && input.non_witness_utxo.is_none() {
        return Err(invalid(
            index,
            "legacy signing requires the full previous transaction",
        ));
    }
    let script = if script.is_p2wsh() {
        let witness = input
            .witness_script
            .as_deref()
            .ok_or_else(|| invalid(index, "missing selected witness script"))?;
        if witness.to_p2wsh().as_script() != script {
            return Err(invalid(
                index,
                "witness script does not match its witness program",
            ));
        }
        witness
    } else {
        script
    };
    if !supported_script(script) {
        return Err(invalid(
            index,
            "selected script contains a code separator or malformed instruction",
        ));
    }
    Ok(())
}

/// Fill selected stacks and finalize the whole transaction without replanning
/// selected inputs. Ordinary sponsor inputs may use their existing assets.
///
/// Every completed input is interpreted after all scriptSigs settle, retaining
/// CTV and annex semantics. The caller's PSBT is unchanged on any error.
pub fn finalize_selected<C: Verification>(
    psbt: &mut Psbt,
    secp: &Secp256k1<C>,
    recipes: &BTreeMap<usize, SatisfactionRecipe>,
) -> Result<(), SelectedError> {
    prevouts(psbt)?;
    let mut candidate = psbt.clone();
    for (&index, recipe) in recipes {
        let input = candidate
            .inputs
            .get_mut(index)
            .ok_or_else(|| invalid(index, "input index out of bounds"))?;
        let finalized = input.final_script_sig.is_some() || input.final_script_witness.is_some();
        let existing_sig = input
            .final_script_sig
            .as_deref()
            .map(script_stack)
            .transpose()
            .map_err(|_| invalid(index, "final scriptSig is not minimally push-only"))?
            .unwrap_or_default();
        let existing_witness = input
            .final_script_witness
            .as_ref()
            .map(Witness::to_vec)
            .unwrap_or_default();
        let script_sig = resolve_stack(
            input,
            index,
            &recipe.script_sig,
            finalized.then_some(existing_sig.as_slice()),
        )?;
        let witness = resolve_stack(
            input,
            index,
            &recipe.witness,
            finalized.then_some(existing_witness.as_slice()),
        )?;
        let script_sig = minimal_script(&script_sig)
            .map_err(|_| invalid(index, "selected scriptSig element exceeds push size"))?;
        // A Some(empty) scriptSig marks even an empty satisfaction as fixed.
        input.final_script_sig = Some(script_sig);
        input.final_script_witness = (!witness.is_empty()).then(|| Witness::from_slice(&witness));
    }
    let mut candidate = finalize::finalize(candidate, secp)
        .map_err(|(_, errors)| SelectedError::Finalization(errors))?;
    for &index in recipes.keys() {
        let original = std::mem::take(&mut candidate.inputs[index]);
        // An empty bare satisfaction has no witness. Its Some(empty)
        // scriptSig remains the PSBT's only evidence of finalization.
        let witness_marks_complete = original.final_script_witness.is_some();
        candidate.inputs[index] = Input {
            non_witness_utxo: original.non_witness_utxo,
            witness_utxo: original.witness_utxo,
            final_script_sig: original
                .final_script_sig
                .filter(|script| !script.is_empty() || !witness_marks_complete),
            final_script_witness: original.final_script_witness,
            proprietary: original.proprietary,
            unknown: original.unknown,
            ..Default::default()
        };
    }
    *psbt = candidate;
    Ok(())
}

fn resolve_stack(
    input: &Input,
    index: usize,
    recipe: &[StackElement],
    existing: Option<&[Vec<u8>]>,
) -> Result<Vec<Vec<u8>>, SelectedError> {
    if existing.is_some_and(|stack| stack.len() != recipe.len()) {
        return Err(invalid(
            index,
            "final stack length differs from the selected recipe",
        ));
    }
    recipe
        .iter()
        .enumerate()
        .map(|(position, element)| {
            let value = existing
                .map(|stack| stack[position].clone())
                .or_else(|| match element {
                    StackElement::Literal(bytes) => Some(bytes.clone()),
                    StackElement::EcdsaSignature(key) => {
                        input.partial_sigs.get(key).map(|sig| sig.to_vec())
                    }
                    StackElement::SchnorrSignature {
                        key,
                        leaf: Some(leaf),
                    } => input
                        .tap_script_sigs
                        .get(&(*key, *leaf))
                        .map(|sig| sig.to_vec()),
                    StackElement::SchnorrSignature { leaf: None, .. } => {
                        input.tap_key_sig.map(|sig| sig.to_vec())
                    }
                    StackElement::Preimage(hash) => match hash {
                        HashRequirement::Sha256(hash) => input.sha256_preimages.get(hash).cloned(),
                        HashRequirement::Hash256(hash) => {
                            input.hash256_preimages.get(hash).cloned()
                        }
                        HashRequirement::Ripemd160(hash) => {
                            input.ripemd160_preimages.get(hash).cloned()
                        }
                        HashRequirement::Hash160(hash) => {
                            input.hash160_preimages.get(hash).cloned()
                        }
                    },
                })
                .ok_or_else(|| SelectedError::MissingAsset {
                    index,
                    element: element.clone(),
                })?;
            let valid = match element {
                StackElement::Literal(bytes) => &value == bytes,
                StackElement::EcdsaSignature(_) => {
                    bitcoin::ecdsa::Signature::from_slice(&value).is_ok()
                }
                StackElement::SchnorrSignature { .. } => {
                    bitcoin::taproot::Signature::from_slice(&value).is_ok()
                }
                StackElement::Preimage(hash) => {
                    value.len() == 32
                        && match hash {
                            HashRequirement::Sha256(hash) => sha256::Hash::hash(&value) == *hash,
                            HashRequirement::Hash256(hash) => sha256d::Hash::hash(&value) == *hash,
                            HashRequirement::Ripemd160(hash) => {
                                ripemd160::Hash::hash(&value) == *hash
                            }
                            HashRequirement::Hash160(hash) => hash160::Hash::hash(&value) == *hash,
                        }
                }
            };
            if !valid {
                return Err(invalid(
                    index,
                    "selected stack element has invalid bytes, encoding, or preimage digest",
                ));
            }
            Ok(value)
        })
        .collect()
}

fn minimal_script(stack: &[Vec<u8>]) -> Result<ScriptBuf, bitcoin::script::PushBytesError> {
    let mut builder = Builder::new();
    for bytes in stack {
        builder = match bytes.as_slice() {
            [] => builder.push_int(0),
            [value @ 1..=16] => builder.push_int(i64::from(*value)),
            [0x81] => builder.push_int(-1),
            _ => builder.push_slice(PushBytesBuf::try_from(bytes.clone())?),
        };
    }
    Ok(builder.into_script())
}

fn script_stack(script: &Script) -> Result<Vec<Vec<u8>>, ()> {
    script
        .instructions_minimal()
        .map(|instruction| match instruction {
            Ok(Instruction::PushBytes(bytes)) => Ok(bytes.as_bytes().to_vec()),
            Ok(Instruction::Op(opcode)) => match opcode.to_u8() {
                0x4f => Ok(vec![0x81]),
                0x51..=0x60 => Ok(vec![opcode.to_u8() - 0x50]),
                _ => Err(()),
            },
            Err(_) => Err(()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scriptsig_selectors_are_minimal_without_changing_stack_bytes() {
        let stack = vec![vec![], vec![1], vec![16], vec![0x81], vec![0]];
        let script = minimal_script(&stack).unwrap();
        assert_eq!(script.as_bytes(), &[0, 0x51, 0x60, 0x4f, 1, 0]);
        assert_eq!(script_stack(&script).unwrap(), stack);
        assert!(script_stack(&ScriptBuf::from_bytes(vec![1, 1])).is_err());
    }
}
