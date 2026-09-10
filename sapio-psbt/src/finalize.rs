//! Finalize supported PSBT scripts while preserving signed annex bytes.
//!
//! Annex-free transactions retain the Miniscript finalizer. For Taproot
//! annexes, the same satisfier and interpreter are used with explicit annex
//! signature hashes. This does not add support for arbitrary TapScript.

use crate::{annex, validate_psbt, PSBTValidationError};
use bitcoin::blockdata::{opcodes::all::OP_CODESEPARATOR, script::Instruction};
use bitcoin::psbt::PartiallySignedTransaction as Psbt;
use bitcoin::secp256k1::{Message, Secp256k1, Verification};
use bitcoin::util::sighash::{Annex, Prevouts, SighashCache};
use bitcoin::util::taproot::{ControlBlock, LeafVersion, TapLeafHash};
use bitcoin::{Script, TxOut, Witness, XOnlyPublicKey};
use miniscript::interpreter::{Interpreter, KeySigPair, SatisfiedConstraint};
use miniscript::psbt::{InputError, PsbtExt, PsbtInputSatisfier};
use miniscript::{Miniscript, Satisfier, Tap};
use sapio_base::util::CTVHash;

/// A finalization failure, retaining the input index when applicable.
#[derive(Debug)]
pub enum FinalizationError {
    /// Invalid public PSBT fields or annex declaration.
    InvalidPSBT(PSBTValidationError),
    /// A standard satisfaction or interpreter failure.
    Miniscript(miniscript::psbt::Error),
    /// An annex or previous-output invariant failed.
    Input { index: usize, reason: &'static str },
}

impl std::fmt::Display for FinalizationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPSBT(error) => std::fmt::Display::fmt(error, formatter),
            Self::Miniscript(error) => std::fmt::Display::fmt(error, formatter),
            Self::Input { index, reason } => write!(formatter, "input {index}: {reason}"),
        }
    }
}

impl std::error::Error for FinalizationError {}

fn input_error(index: usize, reason: &'static str) -> FinalizationError {
    FinalizationError::Input { index, reason }
}

fn miniscript_error(index: usize, error: InputError) -> FinalizationError {
    FinalizationError::Miniscript(miniscript::psbt::Error::InputError(error, index))
}

/// Finalize all inputs, returning successful partial work when any input fails.
///
/// Annexes require native Taproot, the current TapScript leaf version and no
/// code separators. Every executed signature is verified against the exact
/// annex. All satisfactions are checked again after final scriptSigs settle.
#[allow(clippy::result_large_err)]
pub fn finalize<C: Verification>(
    mut psbt: Psbt,
    secp: &Secp256k1<C>,
) -> Result<Psbt, (Psbt, Vec<FinalizationError>)> {
    if let Err(error) = validate_psbt(&psbt) {
        return Err((psbt, vec![FinalizationError::InvalidPSBT(error)]));
    }
    let has_annex = psbt
        .inputs
        .iter()
        .any(|input| annex::get(input).unwrap().is_some());
    if !has_annex {
        return psbt.finalize(secp).map_err(|(psbt, errors)| {
            (
                psbt,
                errors
                    .into_iter()
                    .map(FinalizationError::Miniscript)
                    .collect(),
            )
        });
    }
    let utxos = match previous_outputs(&psbt).and_then(|utxos| {
        check_signature_flags(&psbt)?;
        Ok(utxos)
    }) {
        Ok(utxos) => utxos,
        Err(error) => return Err((psbt, vec![error])),
    };
    let mut errors = Vec::new();
    for native_pass in [false, true] {
        for (index, utxo) in utxos.iter().enumerate() {
            let script = &utxo.script_pubkey;
            let native = script.is_v0_p2wpkh() || script.is_v0_p2wsh() || script.is_v1_p2tr();
            if native == native_pass {
                if let Err(error) = finalize_input(&mut psbt, secp, index, &utxos) {
                    errors.push(error);
                }
            }
        }
    }
    if errors.is_empty() {
        for index in 0..psbt.inputs.len() {
            if let Err(error) = finalize_input(&mut psbt, secp, index, &utxos) {
                errors.push(error);
            }
        }
    }
    if errors.is_empty() {
        Ok(psbt)
    } else {
        Err((psbt, errors))
    }
}

fn previous_outputs(psbt: &Psbt) -> Result<Vec<TxOut>, FinalizationError> {
    psbt.inputs
        .iter()
        .enumerate()
        .map(|(index, input)| {
            let outpoint = psbt.unsigned_tx.input[index].previous_output;
            let full = input
                .non_witness_utxo
                .as_ref()
                .map(|transaction| {
                    if transaction.txid() != outpoint.txid {
                        return Err(input_error(index, "previous transaction ID mismatch"));
                    }
                    transaction
                        .output
                        .get(outpoint.vout as usize)
                        .ok_or_else(|| input_error(index, "previous output index out of bounds"))
                })
                .transpose()?;
            match (input.witness_utxo.as_ref(), full) {
                (Some(witness), Some(full)) if witness != full => {
                    Err(input_error(index, "conflicting previous outputs"))
                }
                (Some(utxo), _) | (None, Some(utxo)) => Ok(utxo.clone()),
                (None, None) => Err(miniscript_error(index, InputError::MissingUtxo)),
            }
        })
        .collect()
}

fn check_signature_flags(psbt: &Psbt) -> Result<(), FinalizationError> {
    for (index, input) in psbt.inputs.iter().enumerate() {
        if let Some(required) = input.sighash_type {
            for got in input
                .partial_sigs
                .values()
                .map(|sig| sig.hash_ty as u32)
                .chain(
                    input
                        .tap_key_sig
                        .iter()
                        .chain(input.tap_script_sigs.values())
                        .map(|sig| sig.hash_ty as u32),
                )
            {
                if required.to_u32() != got {
                    return Err(miniscript_error(
                        index,
                        InputError::SighashMismatch {
                            required: required.to_u32(),
                            got,
                        },
                    ));
                }
            }
        }
    }
    Ok(())
}

fn finalize_input<C: Verification>(
    psbt: &mut Psbt,
    secp: &Secp256k1<C>,
    index: usize,
    utxos: &[TxOut],
) -> Result<(), FinalizationError> {
    let input = &psbt.inputs[index];
    let annex = annex::get(input).map_err(|error| {
        FinalizationError::InvalidPSBT(PSBTValidationError::InvalidAnnex { index, error })
    })?;
    let Some(annex) = annex else {
        return psbt
            .finalize_inp_mut(secp, index)
            .map_err(FinalizationError::Miniscript);
    };
    if !utxos[index].script_pubkey.is_v1_p2tr() {
        return Err(input_error(
            index,
            "annex requires a native Taproot previous output",
        ));
    }
    if input.final_script_sig.is_some() || input.final_script_witness.is_some() {
        let witness = input
            .final_script_witness
            .as_ref()
            .ok_or_else(|| input_error(index, "missing finalized annex witness"))?;
        return verify_annex(psbt, secp, index, utxos, witness, annex);
    }
    let mut best = None;
    if let Some(signature) = input.tap_key_sig {
        let witness = Witness::from_vec(vec![signature.to_vec(), annex.to_vec()]);
        if verify_annex(psbt, secp, index, utxos, &witness, annex).is_ok() {
            best = Some(witness);
        }
    }
    if best.is_none() {
        let satisfier = PsbtInputSatisfier::new(psbt, index);
        for (control, (script, version)) in &input.tap_scripts {
            if *version != LeafVersion::TapScript || control.leaf_version != *version {
                continue;
            }
            let Ok(miniscript) = Miniscript::<XOnlyPublicKey, Tap>::parse_insane(script) else {
                continue;
            };
            let Ok(mut stack) = miniscript.satisfy(&satisfier) else {
                continue;
            };
            stack.push(script.to_bytes());
            stack.push(control.serialize());
            stack.push(annex.to_vec());
            let witness = Witness::from_vec(stack);
            if best.as_ref().map_or(true, |old: &Witness| {
                witness.serialized_len() < old.serialized_len()
            }) && verify_annex(psbt, secp, index, utxos, &witness, annex).is_ok()
            {
                best = Some(witness);
            }
        }
    }
    let witness = best.ok_or_else(|| miniscript_error(index, InputError::CouldNotSatisfyTr))?;
    // Mutate only after the complete candidate has passed interpretation.
    let input = &mut psbt.inputs[index];
    input.final_script_sig = None;
    input.final_script_witness = Some(witness);
    input.partial_sigs.clear();
    input.sighash_type = None;
    input.redeem_script = None;
    input.witness_script = None;
    input.bip32_derivation.clear();
    input.ripemd160_preimages.clear();
    input.sha256_preimages.clear();
    input.hash160_preimages.clear();
    input.hash256_preimages.clear();
    input.tap_key_sig = None;
    input.tap_script_sigs.clear();
    input.tap_scripts.clear();
    input.tap_key_origins.clear();
    input.tap_internal_key = None;
    input.tap_merkle_root = None;
    Ok(())
}

fn verify_annex<C: Verification>(
    psbt: &Psbt,
    secp: &Secp256k1<C>,
    index: usize,
    utxos: &[TxOut],
    witness: &Witness,
    annex: &[u8],
) -> Result<(), FinalizationError> {
    let input = &psbt.inputs[index];
    let script_sig = input.final_script_sig.clone().unwrap_or_default();
    if !script_sig.is_empty() {
        return Err(miniscript_error(index, InputError::NonEmptyScriptSig));
    }
    let mut stack = witness.to_vec();
    if stack.len() < 2 || stack.pop().as_deref() != Some(annex) {
        return Err(input_error(
            index,
            "final witness does not end in its declared annex",
        ));
    }
    let path = if stack.len() == 1 {
        None
    } else {
        let control = ControlBlock::from_slice(stack.last().unwrap())
            .map_err(|_| input_error(index, "invalid Taproot control block"))?;
        if control.leaf_version != LeafVersion::TapScript {
            return Err(input_error(index, "unsupported Taproot leaf version"));
        }
        let script = Script::from(stack[stack.len() - 2].clone());
        let output_key = XOnlyPublicKey::from_slice(&utxos[index].script_pubkey[2..])
            .map_err(|_| input_error(index, "invalid Taproot output key"))?;
        // The interpreter reconstructs Miniscript before checking its proof.
        // Authenticate the actual witness bytes before that normalization.
        if !control.verify_taproot_commitment(secp, output_key, &script) {
            return Err(input_error(
                index,
                "Taproot control block does not commit to witness script",
            ));
        }
        if script.instructions().any(|instruction| match instruction {
            Ok(Instruction::Op(opcode)) => opcode == OP_CODESEPARATOR,
            Err(_) => true,
            _ => false,
        }) {
            return Err(input_error(
                index,
                "annex scripts cannot contain code separators or malformed instructions",
            ));
        }
        Some((
            TapLeafHash::from_script(&script, LeafVersion::TapScript),
            u32::MAX,
        ))
    };
    let stripped = Witness::from_vec(stack);
    let transaction = psbt.clone().extract_tx();
    let interpreter = Interpreter::from_txdata(
        &utxos[index].script_pubkey,
        &script_sig,
        &stripped,
        transaction.lock_time,
        transaction.input[index].sequence,
        transaction.get_ctv_hash(index as u32),
    )
    .map_err(|error| miniscript_error(index, InputError::Interpreter(error)))?;
    let mut sighash = SighashCache::new(&transaction);
    let verify = Box::new(|signature: &KeySigPair| {
        let KeySigPair::Schnorr(key, signature) = signature else {
            return false;
        };
        if input.sighash_type.map_or(false, |required| {
            required.to_u32() != signature.hash_ty as u32
        }) {
            return false;
        }
        let Ok(hash) = sighash.taproot_signature_hash(
            index,
            &Prevouts::All(utxos),
            Some(Annex::new(annex).expect("validated annex")),
            path,
            signature.hash_ty,
        ) else {
            return false;
        };
        let message = Message::from_digest_slice(&hash[..]).expect("32-byte signature hash");
        secp.verify_schnorr(&signature.sig, &message, key).is_ok()
    });
    let satisfier = PsbtInputSatisfier::new(psbt, index);
    for result in interpreter.iter_custom(verify) {
        let constraint =
            result.map_err(|error| miniscript_error(index, InputError::Interpreter(error)))?;
        // The interpreter compares abstract lock values. Completed witnesses
        // must also obey the transaction's version, sequence flags and units.
        let error = match constraint {
            SatisfiedConstraint::RelativeTimeLock { time }
                if !<PsbtInputSatisfier as Satisfier<XOnlyPublicKey>>::check_older(
                    &satisfier, time,
                ) =>
            {
                Some(miniscript::interpreter::Error::RelativeLocktimeNotMet(time))
            }
            SatisfiedConstraint::AbsoluteTimeLock { time }
                if !<PsbtInputSatisfier as Satisfier<XOnlyPublicKey>>::check_after(
                    &satisfier, time,
                ) =>
            {
                Some(miniscript::interpreter::Error::AbsoluteLocktimeNotMet(time))
            }
            _ => None,
        };
        if let Some(error) = error {
            return Err(miniscript_error(index, InputError::Interpreter(error)));
        }
    }
    Ok(())
}
