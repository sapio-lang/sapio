// Copyright Judica, Inc 2026
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Conditional signatures for exact, versioned program instances.
//!
//! Evaluators execute as bounded WebAssembly. Reserved evaluator identities run
//! the supplied program directly; other identities select registered WASM
//! interpreters committed by their exact bytes. The view excludes transaction
//! fields that Taproot SIGHASH_ALL does not commit to.
//! A witness is auxiliary evidence, not a claim that those bytes will appear
//! in the spending transaction. Evaluation has no signing keys, plugin imports,
//! filesystem, network, or clock access.

use bitcoin::bip32::{Xpriv, Xpub};
use bitcoin::blockdata::opcodes::all::OP_CODESEPARATOR;
use bitcoin::blockdata::script::Instruction;
#[cfg(test)]
use bitcoin::hashes::Hash;
use bitcoin::key::TapTweak;
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::{Message, Secp256k1};
use bitcoin::sighash::{Annex, Prevouts, SighashCache};
use bitcoin::taproot::{LeafVersion, TapLeafHash, TapNodeHash};
#[cfg(test)]
use bitcoin::ScriptBuf;
use bitcoin::{OutPoint, TapSighashType, TxOut, XOnlyPublicKey};
use sapio_base::program::{
    program_derivation_path, EvaluatorId, ProgramInstance, WasmVersion, MAX_PROGRAM_ROOT_DEPTH,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

pub use crate::msgs::PSBT;

mod paths;
mod transport;
pub use transport::{ProgramClient, ProgramClientError};
mod wasm;
pub use wasm::{WasmEvaluator, MAX_FUNCTION_SLOTS, MAX_SIGNED_VIEW_BYTES};

/// Maximum auxiliary evidence accepted by the program protocol.
pub const MAX_WITNESS_BYTES: usize = 65_536;

/// The single Taproot signature location requested by the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProgramSpendPath {
    /// Sign the output key after its BIP341 tweak.
    KeyPath,
    /// Sign this known, authenticated tapscript leaf with no code separator.
    ScriptPath(TapLeafHash),
}

/// All data needed to evaluate a predicate and authorize one signature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramSigningRequest {
    /// The evaluator identity, program bytes and fixed parameters.
    pub instance: ProgramInstance,
    /// The transaction input authorized by this request.
    pub input_index: u32,
    /// Auxiliary evidence interpreted only by the selected evaluator.
    #[serde(deserialize_with = "deserialize_witness")]
    pub witness: Vec<u8>,
    /// The sole signature slot this request can modify.
    pub path: ProgramSpendPath,
    /// A structurally valid PSBT with every previous output available.
    pub psbt: PSBT,
}

fn deserialize_witness<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<u8>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = Vec<u8>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "at most {MAX_WITNESS_BYTES} witness bytes")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut bytes = Vec::new();
            while let Some(byte) = sequence.next_element()? {
                if bytes.len() == MAX_WITNESS_BYTES {
                    return Err(serde::de::Error::custom(
                        "program witness exceeds 65536 bytes",
                    ));
                }
                bytes.push(byte);
            }
            Ok(bytes)
        }
    }
    deserializer.deserialize_seq(Visitor)
}

/// An amount and script committed by the signature.
#[derive(Clone, Copy, Debug)]
pub struct SignedOutput<'a> {
    /// Amount in satoshis.
    pub value: u64,
    /// The output's scriptPubKey.
    pub script_pubkey: &'a bitcoin::Script,
}

impl<'a> From<&'a TxOut> for SignedOutput<'a> {
    fn from(output: &'a TxOut) -> Self {
        Self {
            value: output.value.to_sat(),
            script_pubkey: &output.script_pubkey,
        }
    }
}

/// An input's signed fields, excluding scriptSig and witness data.
#[derive(Clone, Copy, Debug)]
pub struct SignedInput<'a> {
    /// The spent transaction output.
    pub previous_output: &'a OutPoint,
    /// The input's sequence number.
    pub sequence: u32,
    /// The previous output's amount and script committed by SIGHASH_ALL.
    pub prevout: SignedOutput<'a>,
}

/// The restricted transaction view available to predicate evaluators.
///
/// There is deliberately no transaction/PSBT accessor, serialization, txid,
/// weight, scriptSig, witness or finalization state. Prior outputs supplied as
/// witness UTXOs need not be authenticated here: a signature for incorrect
/// amounts/scripts cannot validate against the actual spent outputs.
pub struct SignedTransactionView<'a> {
    transaction: &'a bitcoin::Transaction,
    prevouts: &'a [&'a TxOut],
    input_index: u32,
    internal_key: XOnlyPublicKey,
    annex: Option<&'a [u8]>,
}

impl SignedTransactionView<'_> {
    /// Signed transaction version.
    pub fn version(&self) -> i32 {
        self.transaction.version.0
    }

    /// Signed transaction lock time.
    pub fn lock_time(&self) -> u32 {
        self.transaction.lock_time.to_consensus_u32()
    }

    /// The input selected for this signature.
    pub fn input_index(&self) -> u32 {
        self.input_index
    }

    /// The Taproot internal key authenticated against the selected prevout.
    ///
    /// Version two exposes this key. Script-path requests derive it from a
    /// verified control block; key-path requests prove the program-key tweak.
    pub fn internal_key(&self) -> XOnlyPublicKey {
        self.internal_key
    }

    /// The exact annex committed by this input's signature, including `0x50`.
    pub fn annex(&self) -> Option<&[u8]> {
        self.annex
    }

    /// Every input's signed fields, in transaction order.
    pub fn inputs(&self) -> impl ExactSizeIterator<Item = SignedInput<'_>> {
        self.transaction
            .input
            .iter()
            .zip(self.prevouts.iter())
            .map(|(input, prevout)| SignedInput {
                previous_output: &input.previous_output,
                sequence: input.sequence.to_consensus_u32(),
                prevout: (*prevout).into(),
            })
    }

    /// Every signed output, in transaction order.
    pub fn outputs(&self) -> impl ExactSizeIterator<Item = SignedOutput<'_>> {
        self.transaction.output.iter().map(SignedOutput::from)
    }
}

/// An evaluator failed to interpret or complete a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvaluationError(pub String);

impl fmt::Display for EvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for EvaluationError {}

/// A request cannot be evaluated or signed under the program protocol.
#[derive(Debug)]
pub enum ProgramError {
    /// Invalid or underivable public program identity.
    Instance(sapio_base::program::ProgramError),
    /// Invalid PSBT structure.
    Psbt(sapio_psbt::PSBTValidationError),
    /// The root cannot hold the ten-child program derivation.
    RootDepth(u8),
    /// Two registered evaluators claim the same semantic identity.
    DuplicateEvaluator(EvaluatorId),
    /// The identity is reserved for versioned inline WASM programs.
    ReservedEvaluator,
    /// The requested evaluator was not registered by the operator.
    UnknownEvaluator(EvaluatorId),
    /// The evaluator reported a failure.
    Evaluation(EvaluationError),
    /// The predicate evaluated to false.
    Rejected,
    /// Auxiliary witness length exceeds the protocol bound.
    WitnessTooLarge(usize),
    /// The selected input does not exist.
    InputIndex(u32),
    /// A previous output is missing at this input.
    MissingPrevout(usize),
    /// A non-witness transaction does not authenticate the indicated output.
    InvalidNonWitnessPrevout(usize),
    /// Witness and non-witness data disagree about the previous output.
    ConflictingPrevouts(usize),
    /// The selected input already contains finalization data.
    FinalizedInput,
    /// Only an absent declaration or explicit SIGHASH_ALL is supported.
    UnsupportedSighash,
    /// The original evaluator ABI cannot observe or authorize an annex.
    AnnexRequiresV2,
    /// The selected previous output is not a valid P2TR output.
    NotTaproot,
    /// The program key and declared tweak do not match the previous output.
    KeyPathMismatch,
    /// No supplied tapscript and control block authenticate the selected leaf.
    MissingScriptPath,
    /// No supplied Miniscript leaf contains the requested program key.
    MissingProgramLeaf,
    /// Multiple distinct leaves contain the requested program key.
    AmbiguousProgramLeaf,
    /// Optional internal-key or Merkle-root metadata contradicts the proof.
    ConflictingTaprootMetadata,
    /// A selected leaf uses unsupported semantics or OP_CODESEPARATOR.
    UnsupportedScriptPath,
    /// The target slot contains a signature that fails this request's checks.
    ConflictingSignature,
    /// The response changed unauthorized fields or lacks a valid signature.
    InvalidResponse(&'static str),
    /// The requested BIP32 private path could not be derived.
    Derivation(bitcoin::bip32::Error),
    /// The signature hash could not be constructed.
    Sighash(bitcoin::sighash::TaprootError),
}

impl fmt::Display for ProgramError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Instance(error) => error.fmt(formatter),
            Self::Psbt(error) => error.fmt(formatter),
            Self::RootDepth(depth) => write!(
                formatter,
                "program root depth {depth} exceeds {MAX_PROGRAM_ROOT_DEPTH}"
            ),
            Self::DuplicateEvaluator(id) => write!(formatter, "duplicate program evaluator {id:?}"),
            Self::ReservedEvaluator => {
                formatter.write_str("reserved inline WASM identities cannot be registered")
            }
            Self::UnknownEvaluator(id) => write!(formatter, "unknown program evaluator {id:?}"),
            Self::Evaluation(error) => write!(formatter, "program evaluation failed: {error}"),
            Self::Rejected => formatter.write_str("program predicate rejected the transaction"),
            Self::WitnessTooLarge(length) => write!(
                formatter,
                "program witness has {length} bytes; maximum is {MAX_WITNESS_BYTES}"
            ),
            Self::InputIndex(index) => write!(formatter, "no transaction input at index {index}"),
            Self::MissingPrevout(index) => {
                write!(formatter, "missing previous output for input {index}")
            }
            Self::InvalidNonWitnessPrevout(index) => write!(
                formatter,
                "non-witness UTXO does not authenticate input {index}"
            ),
            Self::ConflictingPrevouts(index) => {
                write!(formatter, "conflicting previous outputs for input {index}")
            }
            Self::FinalizedInput => {
                formatter.write_str("selected program input is already finalized")
            }
            Self::UnsupportedSighash => {
                formatter.write_str("program signatures require SIGHASH_ALL")
            }
            Self::AnnexRequiresV2 => {
                formatter.write_str("annex signing requires WASM evaluator version two")
            }
            Self::NotTaproot => {
                formatter.write_str("selected program input must spend a P2TR output")
            }
            Self::KeyPathMismatch => {
                formatter.write_str("program key and tweak do not match the spent output")
            }
            Self::MissingScriptPath => formatter
                .write_str("selected tapscript has no valid commitment to the spent output"),
            Self::MissingProgramLeaf => {
                formatter.write_str("no Miniscript leaf contains the program key")
            }
            Self::AmbiguousProgramLeaf => formatter
                .write_str("multiple leaves contain the program key; select one explicitly"),
            Self::ConflictingTaprootMetadata => formatter
                .write_str("Taproot metadata conflicts with the authenticated spending context"),
            Self::UnsupportedScriptPath => {
                formatter.write_str("program signing requires tapscript without OP_CODESEPARATOR")
            }
            Self::ConflictingSignature => {
                formatter.write_str("existing program signature conflicts with the request")
            }
            Self::InvalidResponse(reason) => {
                write!(formatter, "invalid program signing response: {reason}")
            }
            Self::Derivation(error) => {
                write!(formatter, "program private key derivation failed: {error}")
            }
            Self::Sighash(error) => write!(formatter, "program signature hash failed: {error}"),
        }
    }
}

impl std::error::Error for ProgramError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Instance(error) => Some(error),
            Self::Psbt(error) => Some(error),
            Self::Evaluation(error) => Some(error),
            Self::Derivation(error) => Some(error),
            Self::Sighash(error) => Some(error),
            _ => None,
        }
    }
}

/// A signing root and immutable registry of WASM predicate interpreters.
#[derive(Clone)]
pub struct ProgramOracle {
    root: Xpriv,
    public_root: Xpub,
    evaluators: BTreeMap<EvaluatorId, WasmEvaluator>,
}

impl ProgramOracle {
    /// Register exact WASM interpreters, rejecting duplicate or reserved IDs.
    ///
    /// Reserved identities execute the instance's own program bytes under
    /// their fixed WASM version and cannot be replaced by an interpreter.
    pub fn new(root: Xpriv, evaluators: Vec<WasmEvaluator>) -> Result<Self, ProgramError> {
        if root.depth > MAX_PROGRAM_ROOT_DEPTH {
            return Err(ProgramError::RootDepth(root.depth));
        }
        let mut registry = BTreeMap::new();
        for evaluator in evaluators {
            let id = evaluator.id();
            if id.inline_wasm_version().is_some() {
                return Err(ProgramError::ReservedEvaluator);
            }
            if registry.insert(id, evaluator).is_some() {
                return Err(ProgramError::DuplicateEvaluator(id));
            }
        }
        Ok(Self {
            public_root: Xpub::from_priv(&Secp256k1::new(), &root),
            root,
            evaluators: registry,
        })
    }

    /// The public root used for offline program-policy derivation.
    pub fn public_root(&self) -> Xpub {
        self.public_root
    }

    /// Evaluate the exact program before creating its one requested signature.
    ///
    /// All existing PSBT data is preserved. A valid existing target signature
    /// is returned unchanged; an incompatible signature is an error.
    pub fn sign(&self, request: ProgramSigningRequest) -> Result<Psbt, ProgramError> {
        let (module, program, version) =
            if let Some(version) = request.instance.evaluator().inline_wasm_version() {
                (request.instance.program(), &[][..], version)
            } else {
                let evaluator = self
                    .evaluators
                    .get(&request.instance.evaluator())
                    .ok_or(ProgramError::UnknownEvaluator(request.instance.evaluator()))?;
                (
                    evaluator.module(),
                    request.instance.program(),
                    evaluator.version(),
                )
            };
        let prepared = prepare(&request, &self.public_root)?;
        if version == WasmVersion::V1 && prepared.annex.is_some() {
            return Err(ProgramError::AnnexRequiresV2);
        }
        let existing = target_signature(&request.psbt.0, &request, prepared.program_key);
        if existing.is_some_and(|signature| !prepared.verify(signature)) {
            return Err(ProgramError::ConflictingSignature);
        }
        let view = SignedTransactionView {
            transaction: &request.psbt.0.unsigned_tx,
            prevouts: &prepared.prevouts,
            input_index: request.input_index,
            internal_key: prepared.internal_key,
            annex: prepared.annex,
        };
        if !wasm::evaluate(
            version,
            module,
            program,
            request.instance.parameters(),
            &view,
            &request.witness,
        )
        .map_err(ProgramError::Evaluation)?
        {
            return Err(ProgramError::Rejected);
        }
        if existing.is_some() {
            return Ok(request.psbt.0);
        }
        let secp = Secp256k1::new();
        let derived = self
            .root
            .derive_priv(&secp, &program_derivation_path(request.instance.id()))
            .map_err(ProgramError::Derivation)?;
        let key = derived.to_keypair(&secp);
        let key = match request.path {
            ProgramSpendPath::KeyPath => key
                .tap_tweak(
                    &secp,
                    request.psbt.0.inputs[request.input_index as usize].tap_merkle_root,
                )
                .to_keypair(),
            ProgramSpendPath::ScriptPath(_) => key,
        };
        let signature = bitcoin::taproot::Signature {
            signature: secp.sign_schnorr_no_aux_rand(&prepared.message, &key),
            sighash_type: TapSighashType::All,
        };
        let program_key = prepared.program_key;
        let mut psbt = request.psbt.0;
        put_signature(
            &mut psbt,
            request.input_index,
            request.path,
            program_key,
            signature,
        );
        Ok(psbt)
    }
}

struct Prepared<'a> {
    prevouts: Vec<&'a TxOut>,
    program_key: XOnlyPublicKey,
    verification_key: XOnlyPublicKey,
    message: Message,
    internal_key: XOnlyPublicKey,
    annex: Option<&'a [u8]>,
}

impl Prepared<'_> {
    fn verify(&self, signature: &bitcoin::taproot::Signature) -> bool {
        signature.sighash_type == TapSighashType::All
            && Secp256k1::verification_only()
                .verify_schnorr(&signature.signature, &self.message, &self.verification_key)
                .is_ok()
    }
}

fn previous_outputs(psbt: &Psbt) -> Result<Vec<&TxOut>, ProgramError> {
    psbt.inputs
        .iter()
        .zip(&psbt.unsigned_tx.input)
        .enumerate()
        .map(|(index, (input, txin))| {
            let non_witness = input
                .non_witness_utxo
                .as_ref()
                .map(|transaction| {
                    if transaction.compute_txid() != txin.previous_output.txid {
                        return Err(ProgramError::InvalidNonWitnessPrevout(index));
                    }
                    transaction
                        .output
                        .get(txin.previous_output.vout as usize)
                        .ok_or(ProgramError::InvalidNonWitnessPrevout(index))
                })
                .transpose()?;
            match (input.witness_utxo.as_ref(), non_witness) {
                (Some(witness), Some(non_witness)) if witness != non_witness => {
                    Err(ProgramError::ConflictingPrevouts(index))
                }
                (Some(prevout), _) | (None, Some(prevout)) => Ok(prevout),
                (None, None) => Err(ProgramError::MissingPrevout(index)),
            }
        })
        .collect()
}

fn prepare<'a>(
    request: &'a ProgramSigningRequest,
    root: &Xpub,
) -> Result<Prepared<'a>, ProgramError> {
    if request.witness.len() > MAX_WITNESS_BYTES {
        return Err(ProgramError::WitnessTooLarge(request.witness.len()));
    }
    let psbt = &request.psbt.0;
    sapio_psbt::validate_psbt(psbt).map_err(ProgramError::Psbt)?;
    let index = request.input_index as usize;
    let input = psbt
        .inputs
        .get(index)
        .ok_or(ProgramError::InputIndex(request.input_index))?;
    if input.final_script_sig.is_some() || input.final_script_witness.is_some() {
        return Err(ProgramError::FinalizedInput);
    }
    if input
        .sighash_type
        .is_some_and(|sighash| sighash.taproot_hash_ty() != Ok(TapSighashType::All))
    {
        return Err(ProgramError::UnsupportedSighash);
    }
    let prevouts = previous_outputs(psbt)?;
    let output_script = &prevouts[index].script_pubkey;
    if !output_script.is_p2tr() {
        return Err(ProgramError::NotTaproot);
    }
    let output_key = XOnlyPublicKey::from_slice(&output_script.as_bytes()[2..])
        .map_err(|_| ProgramError::NotTaproot)?;
    let program_key = request
        .instance
        .derive_public_key(root)
        .map_err(ProgramError::Instance)?;
    let secp = Secp256k1::verification_only();
    let (verification_key, internal_key) = match request.path {
        ProgramSpendPath::KeyPath => {
            let (tweaked, _) = program_key.tap_tweak(&secp, input.tap_merkle_root);
            if tweaked.to_x_only_public_key() != output_key {
                return Err(ProgramError::KeyPathMismatch);
            }
            (output_key, program_key)
        }
        ProgramSpendPath::ScriptPath(leaf) => {
            let mut authenticated = None;
            for (control, (script, version)) in &input.tap_scripts {
                if TapLeafHash::from_script(script, *version) != leaf {
                    continue;
                }
                if *version != LeafVersion::TapScript || control.leaf_version != *version {
                    return Err(ProgramError::UnsupportedScriptPath);
                }
                if script.instructions().any(|instruction| {
                    matches!(instruction, Err(_) | Ok(Instruction::Op(OP_CODESEPARATOR)))
                }) {
                    return Err(ProgramError::UnsupportedScriptPath);
                }
                if control.verify_taproot_commitment(&secp, output_key, script) {
                    if let Some(expected_root) = input.tap_merkle_root {
                        let mut root = TapNodeHash::from(leaf);
                        for node in control.merkle_branch.as_slice() {
                            root = TapNodeHash::from_node_hashes(root, *node);
                        }
                        if root != expected_root {
                            return Err(ProgramError::ConflictingTaprootMetadata);
                        }
                    }
                    authenticated = Some(control.internal_key);
                }
            }
            (
                program_key,
                authenticated.ok_or(ProgramError::MissingScriptPath)?,
            )
        }
    };
    if input
        .tap_internal_key
        .is_some_and(|key| key != internal_key)
    {
        return Err(ProgramError::ConflictingTaprootMetadata);
    }
    let annex = sapio_psbt::annex::get(input).map_err(|error| {
        ProgramError::Psbt(sapio_psbt::PSBTValidationError::InvalidAnnex { index, error })
    })?;
    let path = match request.path {
        ProgramSpendPath::KeyPath => None,
        ProgramSpendPath::ScriptPath(leaf) => Some((leaf, u32::MAX)),
    };
    let hash = SighashCache::new(&psbt.unsigned_tx)
        .taproot_signature_hash(
            index,
            &Prevouts::All(&prevouts),
            annex.map(|bytes| Annex::new(bytes).expect("PSBT annex prefix was validated")),
            path,
            TapSighashType::All,
        )
        .map_err(ProgramError::Sighash)?;
    let message = Message::from_digest_slice(&hash[..])
        .expect("Taproot signature hashes contain exactly 32 bytes");
    Ok(Prepared {
        prevouts,
        program_key,
        verification_key,
        message,
        internal_key,
        annex,
    })
}

fn target_signature<'a>(
    psbt: &'a Psbt,
    request: &ProgramSigningRequest,
    program_key: XOnlyPublicKey,
) -> Option<&'a bitcoin::taproot::Signature> {
    let input = psbt.inputs.get(request.input_index as usize)?;
    match request.path {
        ProgramSpendPath::KeyPath => input.tap_key_sig.as_ref(),
        ProgramSpendPath::ScriptPath(leaf) => input.tap_script_sigs.get(&(program_key, leaf)),
    }
}

fn put_signature(
    psbt: &mut Psbt,
    index: u32,
    path: ProgramSpendPath,
    program_key: XOnlyPublicKey,
    signature: bitcoin::taproot::Signature,
) {
    let input = &mut psbt.inputs[index as usize];
    match path {
        ProgramSpendPath::KeyPath => input.tap_key_sig = Some(signature),
        ProgramSpendPath::ScriptPath(leaf) => {
            input.tap_script_sigs.insert((program_key, leaf), signature);
        }
    }
}

/// Verify one signature and require every unrelated PSBT field to be preserved.
///
/// This authenticates the response against the requested public root and
/// exact signature context. It cannot prove that a remote oracle evaluated
/// the program honestly; that remains the emulation trust assumption.
pub fn validate_program_response(
    request: &ProgramSigningRequest,
    response: &Psbt,
    root: &Xpub,
) -> Result<(), ProgramError> {
    let prepared = prepare(request, root)?;
    let signature = target_signature(response, request, prepared.program_key).ok_or(
        ProgramError::InvalidResponse("requested signature is missing"),
    )?;
    if !prepared.verify(signature) {
        return Err(ProgramError::InvalidResponse(
            "requested signature is invalid",
        ));
    }
    let mut expected = request.psbt.0.clone();
    if let Some(existing) = target_signature(&expected, request, prepared.program_key) {
        if existing != signature {
            return Err(ProgramError::InvalidResponse(
                "existing signature was replaced",
            ));
        }
    } else {
        put_signature(
            &mut expected,
            request.input_index,
            request.path,
            prepared.program_key,
            *signature,
        );
    }
    if expected != *response {
        return Err(ProgramError::InvalidResponse(
            "unrelated PSBT data was modified",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod fragments_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tweak_tests;
