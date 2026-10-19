//! Public witness construction for the executed CAT scripts.
//!
//! These routines supply evidence, not authority: the interpreter reconstructs
//! commitments with Script and validates both Schnorr signatures independently.

use super::{invalid, stripped_trigger, Stage};
use bitcoin::consensus::serialize;
use bitcoin::hashes::{sha256, Hash, HashEngine};
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::{schnorr, Message};
use bitcoin::sighash::{Prevouts, SighashCache};
use bitcoin::taproot::{Signature, TapLeafHash};
use bitcoin::{TapSighashType, Transaction, TxOut, XOnlyPublicKey};
use sapio::contract::CompilationError;

const GENERATOR: [u8; 32] = [
    0x79, 0xbe, 0x66, 0x7e, 0xf9, 0xdc, 0xbb, 0xac, 0x55, 0xa0, 0x62, 0x95, 0xce, 0x87, 0x0b, 0x07,
    0x02, 0x9b, 0xfc, 0xdb, 0x2d, 0xce, 0x28, 0xd9, 0x59, 0xf2, 0x81, 0x5b, 0x16, 0xf8, 0x17, 0x98,
];

fn prevouts(psbt: &Psbt) -> Result<Vec<TxOut>, CompilationError> {
    sapio_base::psbt::previous_outputs(psbt)
        .map_err(invalid)?
        .into_iter()
        .map(|output| {
            output
                .cloned()
                .ok_or_else(|| invalid("missing CAT witness prevout"))
        })
        .collect()
}

fn tagged(tag: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let tag = sha256::Hash::hash(tag);
    let mut engine = sha256::Hash::engine();
    engine.input(tag.as_ref());
    engine.input(tag.as_ref());
    for part in parts {
        engine.input(part);
    }
    sha256::Hash::from_engine(engine).to_byte_array()
}

/// The actual wrapper-leaf BIP340 challenge used by the CAT construction.
pub fn challenge(psbt: &Psbt, leaf: TapLeafHash) -> Result<[u8; 32], CompilationError> {
    let prevouts = prevouts(psbt)?;
    let digest = SighashCache::new(&psbt.unsigned_tx)
        .taproot_script_spend_signature_hash(
            0,
            &Prevouts::All(&prevouts),
            leaf,
            TapSighashType::Default,
        )
        .map_err(invalid)?;
    Ok(tagged(
        b"BIP0340/challenge",
        &[&GENERATOR, &GENERATOR, digest.as_ref()],
    ))
}

/// Return the generator-key signature only when its last-byte increment is
/// representable by the upstream construction. Otherwise grind before signing.
pub fn synthetic_signature(
    psbt: &Psbt,
    leaf: TapLeafHash,
) -> Result<Option<[u8; 64]>, CompilationError> {
    let challenge = challenge(psbt, leaf)?;
    if matches!(challenge[31], 0x7f | 0xff) {
        return Ok(None);
    }
    let mut signature = [0; 64];
    signature[..32].copy_from_slice(&GENERATOR);
    signature[32..].copy_from_slice(&challenge);
    signature[63] += 1;
    let prevouts = prevouts(psbt)?;
    let digest = SighashCache::new(&psbt.unsigned_tx)
        .taproot_script_spend_signature_hash(
            0,
            &Prevouts::All(&prevouts),
            leaf,
            TapSighashType::Default,
        )
        .map_err(invalid)?;
    // This also excludes the rare noncanonical scalar/carry cases without
    // assuming that an unreduced SHA256 challenge is a curve scalar.
    let signature_value = schnorr::Signature::from_slice(&signature).map_err(invalid)?;
    let valid = sapio_base::secp::secp()
        .verify_schnorr(
            &signature_value,
            &Message::from_digest(digest.to_byte_array()),
            &XOnlyPublicKey::from_slice(&GENERATOR).map_err(invalid)?,
        )
        .is_ok();
    Ok(valid.then_some(signature))
}

fn digest(bytes: &[u8]) -> Vec<u8> {
    sha256::Hash::hash(bytes).to_byte_array().to_vec()
}

/// Construct the complete initial stack, including a genuine owner's wrapper
/// signature. The script independently binds every supplied component through
/// CAT, SHA256, EQUALVERIFY and CHECKSIG; this function performs no evaluation.
pub fn script_witness(
    stage: Stage,
    psbt: &Psbt,
    leaf: TapLeafHash,
    previous_trigger: Option<&Transaction>,
    owner: Signature,
) -> Result<Vec<Vec<u8>>, CompilationError> {
    let tx = &psbt.unsigned_tx;
    if tx.input.len() != 2 || tx.output.is_empty() {
        return Err(invalid("CAT witness requires two inputs and an output"));
    }
    let prevouts = prevouts(psbt)?;
    let signature = synthetic_signature(psbt, leaf)?
        .ok_or_else(|| invalid("CAT signature requires grinding"))?;
    let mut stack = vec![
        vec![0],
        vec![0],
        serialize(&tx.version),
        serialize(&tx.lock_time),
    ];
    match stage {
        Stage::Trigger | Stage::Cancel => {
            stack.push(digest(
                &tx.input
                    .iter()
                    .flat_map(|input| serialize(&input.previous_output))
                    .collect::<Vec<_>>(),
            ));
        }
        Stage::Complete => {
            stack.push(digest(
                &prevouts
                    .iter()
                    .flat_map(|output| serialize(&output.value))
                    .collect::<Vec<_>>(),
            ));
            stack.push(digest(
                &prevouts
                    .iter()
                    .flat_map(|output| serialize(&output.script_pubkey))
                    .collect::<Vec<_>>(),
            ));
        }
    }
    stack.push(digest(
        &tx.input
            .iter()
            .flat_map(|input| serialize(&input.sequence))
            .collect::<Vec<_>>(),
    ));
    stack.push(vec![2]); // Script path, without an annex.
    if stage == Stage::Complete {
        stack.push(0_u32.to_le_bytes().to_vec());
    }
    stack.push(leaf.to_byte_array().to_vec());
    stack.push(vec![0]); // BIP342 key version.
    stack.push(u32::MAX.to_le_bytes().to_vec()); // No CODESEPARATOR.
    match stage {
        Stage::Trigger | Stage::Cancel => {
            if stage == Stage::Trigger {
                let marker = tx
                    .output
                    .get(1)
                    .ok_or_else(|| invalid("missing destination marker"))?;
                stack.push(marker.script_pubkey.to_bytes());
            }
            stack.push(serialize(&prevouts[0].value));
            stack.push(serialize(&prevouts[0].script_pubkey));
            stack.push(serialize(&prevouts[1].value));
            stack.push(serialize(&prevouts[1].script_pubkey));
        }
        Stage::Complete => {
            let parent = previous_trigger.ok_or_else(|| invalid("missing previous trigger"))?;
            stripped_trigger(parent)?;
            stack.push(serialize(&parent.version));
            for input in &parent.input {
                stack.push(serialize(&input.previous_output));
                stack.push(serialize(&input.sequence));
            }
            stack.push(serialize(&parent.lock_time));
            stack.push(serialize(&prevouts[0].script_pubkey));
            stack.push(serialize(&prevouts[0].value));
            stack.push(tx.output[0].script_pubkey.to_bytes());
            stack.push(serialize(&tx.input[1].previous_output));
        }
    }
    let mut original = signature;
    original[63] -= 1;
    stack.push(original[..63].to_vec());
    stack.push(vec![original[63]]);
    stack.push(vec![signature[63]]);
    stack.push(owner.to_vec());
    Ok(stack)
}
