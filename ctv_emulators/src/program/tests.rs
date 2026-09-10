use super::*;
use bitcoin::blockdata::opcodes::all::{OP_CHECKSIG, OP_DROP};
use bitcoin::blockdata::script::Builder;
use bitcoin::hashes::{sha256, Hash};
use bitcoin::util::psbt::raw;
use bitcoin::util::taproot::TaprootBuilder;
use bitcoin::{Network, Transaction, TxIn, Witness};
use std::sync::atomic::{AtomicUsize, Ordering};

struct PayAtLeast {
    calls: Arc<AtomicUsize>,
}

fn evaluator_id() -> EvaluatorId {
    EvaluatorId(sha256::Hash::hash(b"program signing test evaluator v1"))
}

impl ProgramEvaluator for PayAtLeast {
    fn id(&self) -> EvaluatorId {
        evaluator_id()
    }

    fn evaluate(
        &self,
        program: &[u8],
        parameters: &[u8],
        view: &SignedTransactionView<'_>,
        witness: &[u8],
    ) -> Result<bool, EvaluationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if program != b"pay-at-least" || parameters.len() != 8 || witness.len() != 4 {
            return Err(EvaluationError("invalid program or evidence".into()));
        }
        assert_eq!(view.input_index(), 1);
        assert_eq!(view.inputs().len(), 2);
        let input = view.inputs().nth(1).unwrap();
        assert_eq!(input.previous_output.vout, 0);
        assert_eq!(input.prevout.value, 10_000);
        assert_eq!(input.sequence, 0xffff_fffd);
        let index = u32::from_le_bytes(witness.try_into().unwrap()) as usize;
        let minimum = u64::from_le_bytes(parameters.try_into().unwrap());
        Ok(view
            .outputs()
            .nth(index)
            .is_some_and(|output| output.value >= minimum))
    }
}

fn root() -> ExtendedPrivKey {
    ExtendedPrivKey::new_master(Network::Regtest, &[39; 32]).unwrap()
}

fn instance() -> ProgramInstance {
    ProgramInstance::new(
        evaluator_id(),
        b"pay-at-least".to_vec(),
        9_000_u64.to_le_bytes().to_vec(),
    )
    .unwrap()
}

fn oracle() -> (ProgramOracle, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    (
        ProgramOracle::new(
            root(),
            vec![Arc::new(PayAtLeast {
                calls: calls.clone(),
            })],
        )
        .unwrap(),
        calls,
    )
}

fn request(script: Option<Script>) -> ProgramSigningRequest {
    let secp = Secp256k1::new();
    let instance = instance();
    let public_root = ExtendedPubKey::from_priv(&secp, &root());
    let internal = instance.derive_public_key(&public_root).unwrap();
    let mut builder = TaprootBuilder::new();
    if let Some(script) = &script {
        builder = builder.add_leaf(0, script.clone()).unwrap();
    }
    let spend = builder.finalize(&secp, internal).unwrap();
    let mut psbt = PartiallySignedTransaction::from_unsigned_tx(Transaction {
        version: 2,
        lock_time: 500,
        input: vec![
            TxIn {
                previous_output: OutPoint {
                    txid: bitcoin::Txid::from_inner([1; 32]),
                    vout: 2,
                },
                sequence: 100,
                ..TxIn::default()
            },
            TxIn {
                previous_output: OutPoint {
                    txid: bitcoin::Txid::from_inner([2; 32]),
                    vout: 0,
                },
                sequence: 0xffff_fffd,
                ..TxIn::default()
            },
        ],
        output: vec![TxOut {
            value: 9_000,
            script_pubkey: Script::new(),
        }],
    })
    .unwrap();
    psbt.inputs[0].witness_utxo = Some(TxOut {
        value: 1_000,
        script_pubkey: Script::new(),
    });
    psbt.inputs[1].witness_utxo = Some(TxOut {
        value: 10_000,
        script_pubkey: Script::new_v1_p2tr_tweaked(spend.output_key()),
    });
    psbt.inputs[1].tap_merkle_root = spend.merkle_root();
    let path = if let Some(script) = script {
        let leaf = (script, LeafVersion::TapScript);
        let hash = TapLeafHash::from_script(&leaf.0, leaf.1);
        psbt.inputs[1]
            .tap_scripts
            .insert(spend.control_block(&leaf).unwrap(), leaf);
        ProgramSpendPath::ScriptPath(hash)
    } else {
        ProgramSpendPath::KeyPath
    };
    ProgramSigningRequest {
        instance,
        input_index: 1,
        witness: 0_u32.to_le_bytes().to_vec(),
        path,
        psbt: PSBT(psbt),
    }
}

fn leaf() -> Script {
    let key = instance()
        .derive_public_key(&ExtendedPubKey::from_priv(&Secp256k1::new(), &root()))
        .unwrap();
    Builder::new()
        .push_slice(&key.serialize())
        .push_opcode(OP_CHECKSIG)
        .into_script()
}

fn target<'a>(
    request: &ProgramSigningRequest,
    psbt: &'a PartiallySignedTransaction,
) -> &'a SchnorrSig {
    let key = request
        .instance
        .derive_public_key(&oracle().0.public_root())
        .unwrap();
    target_signature(psbt, request, key).unwrap()
}

#[test]
fn selected_input_and_path_are_the_only_modified_signature_slot() {
    for script in [None, Some(leaf())] {
        let (oracle, calls) = oracle();
        let request = request(script);
        let response = oracle.sign(request.clone()).unwrap();
        validate_program_response(&request, &response, &oracle.public_root()).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(response.inputs[0], request.psbt.0.inputs[0]);
        assert_eq!(target(&request, &response).hash_ty, SchnorrSighashType::All);
        match request.path {
            ProgramSpendPath::KeyPath => assert!(response.inputs[1].tap_script_sigs.is_empty()),
            ProgramSpendPath::ScriptPath(_) => assert!(response.inputs[1].tap_key_sig.is_none()),
        }
        let mut repeated = request;
        repeated.psbt = PSBT(response.clone());
        assert_eq!(oracle.sign(repeated.clone()).unwrap(), response);
        validate_program_response(&repeated, &response, &oracle.public_root()).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        // A previously valid signature does not bypass evaluation of the
        // current auxiliary evidence, which is not part of the sighash.
        repeated.witness = 1_u32.to_le_bytes().to_vec();
        assert!(matches!(oracle.sign(repeated), Err(ProgramError::Rejected)));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }
}

#[test]
fn declined_unknown_and_failed_programs_never_return_a_signature() {
    let (oracle, calls) = oracle();
    let mut rejected = request(None);
    rejected.psbt.0.unsigned_tx.output[0].value = 8_999;
    assert!(matches!(oracle.sign(rejected), Err(ProgramError::Rejected)));
    let mut unknown = request(None);
    unknown.instance = ProgramInstance::new(
        EvaluatorId(sha256::Hash::hash(b"unregistered")),
        vec![],
        vec![],
    )
    .unwrap();
    assert!(matches!(
        oracle.sign(unknown),
        Err(ProgramError::UnknownEvaluator(_))
    ));
    let mut malformed = request(None);
    malformed.witness.push(0);
    assert!(matches!(
        oracle.sign(malformed),
        Err(ProgramError::Evaluation(_))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn missing_conflicting_and_unauthenticated_prevouts_fail_before_evaluation() {
    let (oracle, calls) = oracle();
    let base = request(None);
    let mut missing = base.clone();
    missing.psbt.0.inputs[0].witness_utxo = None;
    assert!(matches!(
        oracle.sign(missing),
        Err(ProgramError::MissingPrevout(0))
    ));
    let parent = Transaction {
        version: 1,
        lock_time: 0,
        input: vec![TxIn::default()],
        output: vec![base.psbt.0.inputs[0].witness_utxo.clone().unwrap()],
    };
    let mut authenticated = base;
    authenticated.psbt.0.unsigned_tx.input[0].previous_output = OutPoint {
        txid: parent.txid(),
        vout: 0,
    };
    authenticated.psbt.0.inputs[0].non_witness_utxo = Some(parent.clone());
    let mut conflict = authenticated.clone();
    conflict.psbt.0.inputs[0]
        .witness_utxo
        .as_mut()
        .unwrap()
        .value += 1;
    assert!(matches!(
        oracle.sign(conflict),
        Err(ProgramError::ConflictingPrevouts(0))
    ));
    let mut wrong_txid = authenticated.clone();
    wrong_txid.psbt.0.inputs[0]
        .non_witness_utxo
        .as_mut()
        .unwrap()
        .lock_time = 1;
    assert!(matches!(
        oracle.sign(wrong_txid),
        Err(ProgramError::InvalidNonWitnessPrevout(0))
    ));
    let mut wrong_vout = authenticated.clone();
    wrong_vout.psbt.0.unsigned_tx.input[0].previous_output.vout = 1;
    assert!(matches!(
        oracle.sign(wrong_vout),
        Err(ProgramError::InvalidNonWitnessPrevout(0))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let signed = oracle.sign(authenticated.clone()).unwrap();
    validate_program_response(&authenticated, &signed, &oracle.public_root()).unwrap();
    authenticated.psbt.0.inputs[0].witness_utxo = None;
    let from_parent = oracle.sign(authenticated.clone()).unwrap();
    assert_eq!(
        target(&authenticated, &from_parent),
        target(&authenticated, &signed)
    );
    validate_program_response(&authenticated, &from_parent, &oracle.public_root()).unwrap();
}

#[test]
fn signing_rejects_structural_sighash_finalization_and_bound_violations() {
    let (oracle, calls) = oracle();
    let base = request(None);
    let mut malformed = base.clone();
    malformed.psbt.0.inputs.pop();
    assert!(matches!(oracle.sign(malformed), Err(ProgramError::Psbt(_))));
    let mut malformed = base.clone();
    malformed.psbt.0.outputs.clear();
    assert!(matches!(oracle.sign(malformed), Err(ProgramError::Psbt(_))));
    let mut malformed = base.clone();
    malformed.psbt.0.unsigned_tx.input[0].script_sig = Script::from(vec![0x51]);
    assert!(matches!(oracle.sign(malformed), Err(ProgramError::Psbt(_))));
    let mut malformed = base.clone();
    malformed.psbt.0.unsigned_tx.input[0].witness = Witness::from_vec(vec![vec![1]]);
    assert!(matches!(oracle.sign(malformed), Err(ProgramError::Psbt(_))));
    let mut malformed = base.clone();
    malformed.input_index = 2;
    assert!(matches!(
        oracle.sign(malformed),
        Err(ProgramError::InputIndex(2))
    ));
    for hash_ty in [
        SchnorrSighashType::Default,
        SchnorrSighashType::None,
        SchnorrSighashType::Single,
        SchnorrSighashType::AllPlusAnyoneCanPay,
    ] {
        let mut malformed = base.clone();
        malformed.psbt.0.inputs[1].sighash_type = Some(hash_ty.into());
        assert!(matches!(
            oracle.sign(malformed),
            Err(ProgramError::UnsupportedSighash)
        ));
    }
    let mut finalized = base.clone();
    finalized.psbt.0.inputs[1].final_script_sig = Some(Script::new());
    assert!(matches!(
        oracle.sign(finalized),
        Err(ProgramError::FinalizedInput)
    ));
    let mut finalized = base.clone();
    finalized.psbt.0.inputs[1].final_script_witness = Some(Witness::new());
    assert!(matches!(
        oracle.sign(finalized),
        Err(ProgramError::FinalizedInput)
    ));
    let mut oversized = base.clone();
    oversized.witness = vec![0; MAX_WITNESS_BYTES + 1];
    assert!(serde_json::from_value::<ProgramSigningRequest>(
        serde_json::to_value(&oversized).unwrap()
    )
    .is_err());
    assert!(matches!(
        oracle.sign(oversized),
        Err(ProgramError::WitnessTooLarge(_))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let mut explicit_all = base;
    explicit_all.psbt.0.inputs[1].sighash_type = Some(SchnorrSighashType::All.into());
    oracle.sign(explicit_all).unwrap();
}

#[test]
fn signing_authenticates_key_tweak_and_script_control_block() {
    let (oracle, calls) = oracle();
    let mut not_taproot = request(None);
    not_taproot.psbt.0.inputs[1]
        .witness_utxo
        .as_mut()
        .unwrap()
        .script_pubkey = Script::new();
    assert!(matches!(
        oracle.sign(not_taproot),
        Err(ProgramError::NotTaproot)
    ));
    let mut wrong_key = request(None);
    wrong_key.psbt.0.inputs[1]
        .witness_utxo
        .as_mut()
        .unwrap()
        .script_pubkey = request(Some(leaf())).psbt.0.inputs[1]
        .witness_utxo
        .as_ref()
        .unwrap()
        .script_pubkey
        .clone();
    assert!(matches!(
        oracle.sign(wrong_key),
        Err(ProgramError::KeyPathMismatch)
    ));
    let mut missing_leaf = request(Some(leaf()));
    missing_leaf.psbt.0.inputs[1].tap_scripts.clear();
    assert!(matches!(
        oracle.sign(missing_leaf),
        Err(ProgramError::MissingScriptPath)
    ));
    let mut wrong_control = request(Some(leaf()));
    let (mut control, script) = wrong_control.psbt.0.inputs[1]
        .tap_scripts
        .pop_first()
        .unwrap();
    control.output_key_parity = if control.output_key_parity == bitcoin::secp256k1::Parity::Even {
        bitcoin::secp256k1::Parity::Odd
    } else {
        bitcoin::secp256k1::Parity::Even
    };
    wrong_control.psbt.0.inputs[1]
        .tap_scripts
        .insert(control, script);
    assert!(matches!(
        oracle.sign(wrong_control),
        Err(ProgramError::MissingScriptPath)
    ));
    let code_separator = Builder::new()
        .push_opcode(OP_CODESEPARATOR)
        .push_int(1)
        .into_script();
    assert!(matches!(
        oracle.sign(request(Some(code_separator))),
        Err(ProgramError::UnsupportedScriptPath)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    // A pushed byte is data, not an executed code separator. The signature
    // certifies the predicate even when the script does not use this key.
    let pushed = Builder::new()
        .push_slice(&[OP_CODESEPARATOR.into_u8()])
        .push_opcode(OP_DROP)
        .push_int(1)
        .into_script();
    let request = request(Some(pushed));
    let response = oracle.sign(request.clone()).unwrap();
    validate_program_response(&request, &response, &oracle.public_root()).unwrap();
}

#[test]
fn finalization_metadata_on_other_inputs_cannot_change_evaluation_or_signature() {
    let (oracle, calls) = oracle();
    let original = request(None);
    let first = oracle.sign(original.clone()).unwrap();
    let mut with_metadata = original;
    with_metadata.psbt.0.inputs[0].final_script_sig = Some(Script::from(vec![0x51]));
    with_metadata.psbt.0.inputs[0].final_script_witness =
        Some(Witness::from_vec(vec![vec![1, 2, 3]]));
    with_metadata.psbt.0.inputs[0].unknown.insert(
        raw::Key {
            type_value: 0xfa,
            key: vec![1],
        },
        vec![2],
    );
    let second = oracle.sign(with_metadata.clone()).unwrap();
    assert_eq!(
        target(&with_metadata, &first),
        target(&with_metadata, &second)
    );
    assert_eq!(second.inputs[0], with_metadata.psbt.0.inputs[0]);
    validate_program_response(&with_metadata, &second, &oracle.public_root()).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn signed_field_mutations_invalidate_both_signature_forms() {
    type Mutation = fn(&mut PartiallySignedTransaction);
    let mutations: &[Mutation] = &[
        |psbt| psbt.unsigned_tx.version += 1,
        |psbt| psbt.unsigned_tx.lock_time += 1,
        |psbt| psbt.unsigned_tx.input[0].previous_output.vout += 1,
        |psbt| psbt.unsigned_tx.input[0].sequence += 1,
        |psbt| psbt.inputs[0].witness_utxo.as_mut().unwrap().value += 1,
        |psbt| {
            psbt.inputs[0].witness_utxo.as_mut().unwrap().script_pubkey = Script::from(vec![0x51])
        },
        |psbt| psbt.unsigned_tx.output[0].value += 1,
        |psbt| psbt.unsigned_tx.output[0].script_pubkey = Script::from(vec![0x51]),
    ];
    for script in [None, Some(leaf())] {
        let (oracle, _) = oracle();
        let request = request(script);
        let response = oracle.sign(request.clone()).unwrap();
        for mutate in mutations {
            let mut altered_request = request.clone();
            let mut altered_response = response.clone();
            mutate(&mut altered_request.psbt.0);
            mutate(&mut altered_response);
            assert!(matches!(
                validate_program_response(
                    &altered_request,
                    &altered_response,
                    &oracle.public_root()
                ),
                Err(ProgramError::InvalidResponse(
                    "requested signature is invalid"
                ))
            ));
        }
    }
}

#[test]
fn responses_must_preserve_all_metadata_and_unrelated_signature_slots() {
    for script in [None, Some(leaf())] {
        let (oracle, _) = oracle();
        let request = request(script);
        let response = oracle.sign(request.clone()).unwrap();
        let signature = *target(&request, &response);
        let mut on_other_input = response.clone();
        on_other_input.inputs[0].tap_key_sig = Some(signature);
        assert!(
            validate_program_response(&request, &on_other_input, &oracle.public_root()).is_err()
        );
        let mut extra_key = response.clone();
        extra_key.inputs[1].tap_script_sigs.insert(
            (
                root().to_keypair(&Secp256k1::new()).x_only_public_key().0,
                TapLeafHash::from_inner([2; 32]),
            ),
            signature,
        );
        assert!(validate_program_response(&request, &extra_key, &oracle.public_root()).is_err());
        let mut metadata = response.clone();
        metadata.unknown.insert(
            raw::Key {
                type_value: 0xfa,
                key: vec![1],
            },
            vec![2],
        );
        assert!(validate_program_response(&request, &metadata, &oracle.public_root()).is_err());
        let mut missing_input = response.clone();
        missing_input.inputs.pop();
        assert!(
            validate_program_response(&request, &missing_input, &oracle.public_root()).is_err()
        );
        let mut finalized = response.clone();
        finalized.inputs[1].final_script_witness = Some(Witness::new());
        assert!(validate_program_response(&request, &finalized, &oracle.public_root()).is_err());
        assert!(
            validate_program_response(&request, &request.psbt.0, &oracle.public_root()).is_err()
        );
        let wrong_root = ExtendedPubKey::from_priv(
            &Secp256k1::new(),
            &ExtendedPrivKey::new_master(Network::Regtest, &[40; 32]).unwrap(),
        );
        assert!(validate_program_response(&request, &response, &wrong_root).is_err());
    }
}

#[test]
fn conflicting_target_signatures_are_rejected_and_unrelated_ones_preserved() {
    for script in [None, Some(leaf())] {
        let (oracle, calls) = oracle();
        let mut request = request(script);
        let response = oracle.sign(request.clone()).unwrap();
        let key = request
            .instance
            .derive_public_key(&oracle.public_root())
            .unwrap();
        let mut bad_signature = *target(&request, &response);
        bad_signature.hash_ty = SchnorrSighashType::Default;
        put_signature(&mut request.psbt.0, 1, request.path, key, bad_signature);
        assert!(matches!(
            oracle.sign(request.clone()),
            Err(ProgramError::ConflictingSignature)
        ));
        assert!(validate_program_response(&request, &response, &oracle.public_root()).is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // An unrelated partial signature remains caller-owned data.
        request.psbt = PSBT(response.clone());
        request.psbt.0.inputs[0].tap_key_sig = Some(bad_signature);
        let preserved = oracle.sign(request.clone()).unwrap();
        assert_eq!(preserved, request.psbt.0);
        validate_program_response(&request, &preserved, &oracle.public_root()).unwrap();
    }
}

#[test]
fn registry_depth_and_request_serialization_have_no_implicit_fallback() {
    let evaluator = Arc::new(PayAtLeast {
        calls: Arc::new(AtomicUsize::new(0)),
    });
    assert!(matches!(
        ProgramOracle::new(root(), vec![evaluator.clone(), evaluator]),
        Err(ProgramError::DuplicateEvaluator(_))
    ));
    let mut deep = root();
    deep.depth = MAX_PROGRAM_ROOT_DEPTH;
    ProgramOracle::new(deep, vec![]).unwrap();
    deep.depth += 1;
    assert!(matches!(
        ProgramOracle::new(deep, vec![]),
        Err(ProgramError::RootDepth(_))
    ));
    for request in [request(None), request(Some(leaf()))] {
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(
            serde_json::from_value::<ProgramSigningRequest>(value.clone()).unwrap(),
            request
        );
        let mut unknown = value;
        unknown
            .as_object_mut()
            .unwrap()
            .insert("annex".into(), serde_json::json!([1]));
        assert!(serde_json::from_value::<ProgramSigningRequest>(unknown).is_err());
    }
}
