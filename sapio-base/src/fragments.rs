//! Public construction of the distributed WASM covenant fragments.
//!
//! Evaluation runs the committed guest bytes. The native template hash helper
//! constructs authorization messages; it does not replace WASM evaluation.

use crate::program::ProgramInstance;
use bitcoin::consensus::Encodable;
use bitcoin::hashes::{sha256, Hash, HashEngine};
use bitcoin::secp256k1::{schnorr::Signature, Parity, Scalar};
use bitcoin::util::sighash::Annex;
use bitcoin::{Transaction, XOnlyPublicKey};
use std::fmt;

/// The exact distributed BIP446 hash-equality evaluator (WASM version two).
pub const TEMPLATEHASH_WASM: &[u8] = include_bytes!("../../evaluators/artifacts/templatehash.wasm");
/// The exact distributed template-signature evaluator (WASM version two).
pub const TEMPLATE_AUTHORIZATION_WASM: &[u8] =
    include_bytes!("../../evaluators/artifacts/template_authorization.wasm");
const _: () = assert!(TEMPLATEHASH_WASM.len() <= crate::program::MAX_PROGRAM_BYTES);
const _: () = assert!(TEMPLATE_AUTHORIZATION_WASM.len() <= crate::program::MAX_PROGRAM_BYTES);

/// Select the verification key for a template authorization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateKey {
    /// Commit a specific authorization key in the program parameters.
    Pinned(XOnlyPublicKey),
    /// Use the authenticated Taproot internal key of the selected input.
    InternalKey,
    /// Witness proves an additive opening of the selected output key.
    KnownTweak,
}

/// Require the exact BIP446 template hash, with no auxiliary witness.
pub fn templatehash_wasm_instance(expected: sha256::Hash) -> ProgramInstance {
    ProgramInstance::wasm_v2(TEMPLATEHASH_WASM.to_vec(), expected.as_ref().to_vec())
        .expect("distributed module and fixed parameters fit the program bounds")
}

/// Authorize the current template using CSFS and the selected key source.
///
/// Pinned/internal-key modes take a raw 64-byte BIP340 signature as witness.
/// Known-tweak mode takes [`known_tweak_witness`]. Empty signatures reject.
pub fn template_authorization_wasm_instance(key: TemplateKey) -> ProgramInstance {
    let parameters = match key {
        TemplateKey::Pinned(key) => {
            let mut bytes = vec![0];
            bytes.extend_from_slice(&key.serialize());
            bytes
        }
        TemplateKey::InternalKey => vec![1],
        TemplateKey::KnownTweak => vec![2],
    };
    ProgramInstance::wasm_v2(TEMPLATE_AUTHORIZATION_WASM.to_vec(), parameters)
        .expect("distributed module and fixed parameters fit the program bounds")
}

/// Encode `P[32] || t[32] || parity[1] || signature[0 or 64]`.
///
/// The guest verifies `Q = lift_x(P) + t*G` against the spent output key Q,
/// then verifies the signature under P over the computed template hash.
/// This does not assert that t is a particular BIP32 or TapTweak derivation.
pub fn known_tweak_witness(
    key: XOnlyPublicKey,
    tweak: Scalar,
    parity: Parity,
    signature: Option<&Signature>,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(65 + signature.map_or(0, |_| 64));
    bytes.extend_from_slice(&key.serialize());
    bytes.extend_from_slice(&tweak.to_be_bytes());
    bytes.push(parity.to_i32() as u8);
    if let Some(signature) = signature {
        bytes.extend_from_slice(signature.as_ref());
    }
    bytes
}

/// Invalid context for computing a BIP446 authorization message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateHashError {
    /// The selected transaction input does not exist.
    InputIndex,
    /// A present annex must be nonempty and start with `0x50`.
    Annex,
}

impl fmt::Display for TemplateHashError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InputIndex => "template hash input index is out of range",
            Self::Annex => "template hash annex must begin with 0x50",
        })
    }
}

impl std::error::Error for TemplateHashError {}

/// Compute the exact BIP446 template hash for an authorization signature.
///
/// Annex absence is committed. ScriptSigs, input outpoints, and input amounts
/// are not committed; signatures can be reused across matching templates.
pub fn template_hash(
    transaction: &Transaction,
    input_index: u32,
    annex: Option<&[u8]>,
) -> Result<sha256::Hash, TemplateHashError> {
    if input_index as usize >= transaction.input.len() {
        return Err(TemplateHashError::InputIndex);
    }
    let annex = annex
        .map(Annex::new)
        .transpose()
        .map_err(|_| TemplateHashError::Annex)?;
    let tag = sha256::Hash::hash(b"TemplateHash");
    let mut hash = sha256::Hash::engine();
    hash.input(tag.as_ref());
    hash.input(tag.as_ref());
    hash.input(&transaction.version.to_le_bytes());
    hash.input(&transaction.lock_time.to_le_bytes());
    let mut sequences = sha256::Hash::engine();
    for input in &transaction.input {
        sequences.input(&input.sequence.to_le_bytes());
    }
    hash.input(sha256::Hash::from_engine(sequences).as_ref());
    let mut outputs = sha256::Hash::engine();
    for output in &transaction.output {
        output
            .consensus_encode(&mut outputs)
            .expect("hash engines accept every byte");
    }
    hash.input(sha256::Hash::from_engine(outputs).as_ref());
    hash.input(&[u8::from(annex.is_some())]);
    hash.input(&input_index.to_le_bytes());
    if let Some(annex) = annex {
        let mut annex_hash = sha256::Hash::engine();
        annex
            .consensus_encode(&mut annex_hash)
            .expect("hash engines accept every byte");
        hash.input(sha256::Hash::from_engine(annex_hash).as_ref());
    }
    Ok(sha256::Hash::from_engine(hash))
}
