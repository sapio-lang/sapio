use super::*;
use bitcoin::blockdata::opcodes::all::OP_CHECKSIG;
use bitcoin::blockdata::script::Builder;
use bitcoin::secp256k1::{Parity, Scalar, SecretKey};
use bitcoin::util::taproot::{TapTweakHash, TaprootBuilder};
use bitcoin::{KeyPair, Network, Transaction, TxIn};
use sapio_base::fragments::{
    known_tweak_witness, template_authorization_wasm_instance, template_hash,
    templatehash_wasm_instance, TemplateKey, TEMPLATE_AUTHORIZATION_WASM,
};

fn root() -> ExtendedPrivKey {
    ExtendedPrivKey::new_master(Network::Regtest, &[68; 32]).unwrap()
}

fn owner() -> KeyPair {
    KeyPair::from_secret_key(
        &Secp256k1::new(),
        &SecretKey::from_slice(&[71; 32]).unwrap(),
    )
}

fn transaction() -> Transaction {
    Transaction {
        version: 2,
        lock_time: 40,
        input: vec![
            TxIn {
                previous_output: OutPoint {
                    txid: bitcoin::Txid::from_inner([21; 32]),
                    vout: 2,
                },
                sequence: 42,
                ..TxIn::default()
            },
            TxIn {
                previous_output: OutPoint {
                    txid: bitcoin::Txid::from_inner([22; 32]),
                    vout: 3,
                },
                sequence: 0xffff_fffd,
                ..TxIn::default()
            },
        ],
        output: vec![TxOut {
            value: 7_000,
            script_pubkey: Builder::new().push_int(1).into_script(),
        }],
    }
}

fn request(
    instance: ProgramInstance,
    script_path: bool,
    annex: Option<Vec<u8>>,
) -> ProgramSigningRequest {
    let secp = Secp256k1::new();
    let key = instance
        .derive_public_key(&ExtendedPubKey::from_priv(&secp, &root()))
        .unwrap();
    let mut builder = TaprootBuilder::new();
    let script = Builder::new()
        .push_slice(&key.serialize())
        .push_opcode(OP_CHECKSIG)
        .into_script();
    let internal = if script_path {
        builder = builder.add_leaf(0, script.clone()).unwrap();
        owner().x_only_public_key().0
    } else {
        key
    };
    let spend = builder.finalize(&secp, internal).unwrap();
    let mut psbt = PartiallySignedTransaction::from_unsigned_tx(transaction()).unwrap();
    for (index, input) in psbt.inputs.iter_mut().enumerate() {
        input.witness_utxo = Some(TxOut {
            value: if index == 0 { 3_000 } else { 6_000 },
            script_pubkey: Script::new_v1_p2tr_tweaked(spend.output_key()),
        });
    }
    psbt.inputs[1].tap_merkle_root = spend.merkle_root();
    let path = if script_path {
        let leaf = (script, LeafVersion::TapScript);
        let hash = TapLeafHash::from_script(&leaf.0, leaf.1);
        psbt.inputs[1]
            .tap_scripts
            .insert(spend.control_block(&leaf).unwrap(), leaf);
        ProgramSpendPath::ScriptPath(hash)
    } else {
        ProgramSpendPath::KeyPath
    };
    sapio_psbt::annex::set(&mut psbt.inputs[1], annex).unwrap();
    ProgramSigningRequest {
        instance,
        input_index: 1,
        witness: vec![],
        path,
        psbt: PSBT(psbt),
    }
}

fn signature(
    request: &ProgramSigningRequest,
    key: &KeyPair,
) -> bitcoin::secp256k1::schnorr::Signature {
    let hash = template_hash(
        &request.psbt.0.unsigned_tx,
        request.input_index,
        sapio_psbt::annex::get(&request.psbt.0.inputs[1]).unwrap(),
    )
    .unwrap();
    Secp256k1::new()
        .sign_schnorr_no_aux_rand(&Message::from_digest_slice(hash.as_ref()).unwrap(), key)
}

fn program_keypair(instance: &ProgramInstance) -> KeyPair {
    let secp = Secp256k1::new();
    root()
        .derive_priv(&secp, &program_derivation_path(instance.id()))
        .unwrap()
        .to_keypair(&secp)
}

fn oracle() -> ProgramOracle {
    ProgramOracle::new(root(), vec![]).unwrap()
}

#[test]
fn templatehash_guest_commits_fields_and_annex_but_allows_legacy_companion_inputs() {
    for annex in [
        None,
        Some(vec![0x50]),
        Some(vec![0x50; 253]),
        Some(vec![0x50; 65_536]),
    ] {
        let expected = template_hash(&transaction(), 1, annex.as_deref()).unwrap();
        let mut base = request(templatehash_wasm_instance(expected), false, annex.clone());
        // TEMPLATEHASH has no scriptSig commitment and no CTV witness-only restriction.
        base.psbt.0.inputs[0]
            .witness_utxo
            .as_mut()
            .unwrap()
            .script_pubkey = Script::new();
        let oracle = oracle();
        let signed = oracle.sign(base.clone()).unwrap();
        validate_program_response(&base, &signed, &oracle.public_root()).unwrap();
        let mut changes = Vec::new();
        let mut changed = base.clone();
        changed.psbt.0.unsigned_tx.version += 1;
        changes.push(changed);
        let mut changed = base.clone();
        changed.psbt.0.unsigned_tx.lock_time += 1;
        changes.push(changed);
        let mut changed = base.clone();
        changed.psbt.0.unsigned_tx.input[0].sequence -= 1;
        changes.push(changed);
        let mut changed = base.clone();
        changed.psbt.0.unsigned_tx.output[0].value += 1;
        changes.push(changed);
        let mut changed = base.clone();
        changed.psbt.0.unsigned_tx.output[0].script_pubkey = Script::new();
        changes.push(changed);
        let mut changed = base.clone();
        sapio_psbt::annex::set(
            &mut changed.psbt.0.inputs[1],
            if annex.is_some() {
                None
            } else {
                Some(vec![0x50])
            },
        )
        .unwrap();
        changes.push(changed);
        if let Some(mut bytes) = annex.clone() {
            if bytes.len() > 1 {
                bytes[1] ^= 1;
            } else {
                bytes.push(1);
            }
            let mut changed = base.clone();
            sapio_psbt::annex::set(&mut changed.psbt.0.inputs[1], Some(bytes)).unwrap();
            changes.push(changed);
        }
        for changed in changes {
            assert!(matches!(oracle.sign(changed), Err(ProgramError::Rejected)));
        }
        let mut rebound = base.clone();
        rebound.psbt.0.unsigned_tx.input[1].previous_output.txid =
            bitcoin::Txid::from_inner([24; 32]);
        let rebound_signature = oracle.sign(rebound).unwrap();
        assert_ne!(
            signed.inputs[1].tap_key_sig,
            rebound_signature.inputs[1].tap_key_sig
        );
    }
}

#[test]
fn csfs_template_authorization_rebinds_and_preserves_terminal_signature_failures() {
    let owner = owner();
    let mut base = request(
        template_authorization_wasm_instance(TemplateKey::Pinned(owner.x_only_public_key().0)),
        false,
        Some(vec![0x50, 1, 2]),
    );
    base.witness = signature(&base, &owner).as_ref().to_vec();
    let oracle = oracle();
    let signed = oracle.sign(base.clone()).unwrap();
    let mut rebound = base.clone();
    rebound.psbt.0.unsigned_tx.input[1].previous_output.txid = bitcoin::Txid::from_inner([25; 32]);
    let second = oracle.sign(rebound.clone()).unwrap();
    assert_ne!(signed.inputs[1].tap_key_sig, second.inputs[1].tap_key_sig);
    validate_program_response(&rebound, &second, &oracle.public_root()).unwrap();
    for witness in [
        vec![0; 63],
        vec![0; 64],
        vec![0; 65],
        vec![0; MAX_WITNESS_BYTES],
    ] {
        let mut changed = base.clone();
        changed.witness = witness;
        assert!(matches!(
            oracle.sign(changed),
            Err(ProgramError::Evaluation(_))
        ));
    }
    let mut empty = base.clone();
    empty.witness.clear();
    assert!(matches!(oracle.sign(empty), Err(ProgramError::Rejected)));
    let mut wrong_message = base.clone();
    wrong_message.psbt.0.unsigned_tx.output[0].value += 1;
    assert!(matches!(
        oracle.sign(wrong_message),
        Err(ProgramError::Evaluation(_))
    ));
    let mut already_signed = base.clone();
    already_signed.psbt = PSBT(signed);
    already_signed.witness[0] ^= 1;
    assert!(matches!(
        oracle.sign(already_signed),
        Err(ProgramError::Evaluation(_))
    ));
}

#[test]
fn internal_key_is_authenticated_from_control_block_or_keypath_tweak() {
    for script_path in [false, true] {
        let mut base = request(
            template_authorization_wasm_instance(TemplateKey::InternalKey),
            script_path,
            Some(vec![0x50, 9]),
        );
        let signing_key = if script_path {
            owner()
        } else {
            program_keypair(&base.instance)
        };
        base.witness = signature(&base, &signing_key).as_ref().to_vec();
        let oracle = oracle();
        let signed = oracle.sign(base.clone()).unwrap();
        validate_program_response(&base, &signed, &oracle.public_root()).unwrap();
        let mut forged_metadata = base.clone();
        forged_metadata.psbt.0.inputs[1].tap_internal_key =
            Some(root().to_keypair(&Secp256k1::new()).x_only_public_key().0);
        assert!(matches!(
            oracle.sign(forged_metadata),
            Err(ProgramError::ConflictingTaprootMetadata)
        ));
        if script_path {
            let mut forged = base.clone();
            let (mut control, leaf) = forged.psbt.0.inputs[1].tap_scripts.pop_first().unwrap();
            control.internal_key = root().to_keypair(&Secp256k1::new()).x_only_public_key().0;
            forged.psbt.0.inputs[1].tap_scripts.insert(control, leaf);
            assert!(matches!(
                oracle.sign(forged),
                Err(ProgramError::MissingScriptPath)
            ));
            let mut forged = base.clone();
            forged.psbt.0.inputs[1].tap_merkle_root = Some(TapBranchHash::from_inner([0; 32]));
            assert!(matches!(
                oracle.sign(forged),
                Err(ProgramError::ConflictingTaprootMetadata)
            ));
        }
    }
}

#[test]
fn known_tweak_and_untweaked_signature_authorize_a_keypath_without_control_block() {
    let mut base = request(
        template_authorization_wasm_instance(TemplateKey::KnownTweak),
        false,
        Some(vec![0x50, 8]),
    );
    let keypair = program_keypair(&base.instance);
    let (key, _) = keypair.x_only_public_key();
    let tweak = TapTweakHash::from_key_and_tweak(key, None).to_scalar();
    let (_, parity) = key.tap_tweak(&Secp256k1::new(), None);
    base.witness = known_tweak_witness(key, tweak, parity, Some(&signature(&base, &keypair)));
    assert!(base.psbt.0.inputs[1].tap_internal_key.is_none());
    assert!(base.psbt.0.inputs[1].tap_scripts.is_empty());
    let oracle = oracle();
    let signed = oracle.sign(base.clone()).unwrap();
    validate_program_response(&base, &signed, &oracle.public_root()).unwrap();
    for offset in [0, 31, 32, 63, 64, 65, 96, 128] {
        let mut changed = base.clone();
        changed.witness[offset] ^= 1;
        assert!(matches!(
            oracle.sign(changed),
            Err(ProgramError::Evaluation(_))
        ));
    }
    let mut overflow = base.clone();
    overflow.witness[32..64].fill(0xff);
    assert!(matches!(
        oracle.sign(overflow),
        Err(ProgramError::Evaluation(_))
    ));
    for length in [0, 32, 64, 66, 128, 130] {
        let mut changed = base.clone();
        changed.witness.resize(length, 0);
        assert!(matches!(
            oracle.sign(changed),
            Err(ProgramError::Evaluation(_))
        ));
    }
    let mut empty = base;
    empty.witness.truncate(65);
    assert!(matches!(oracle.sign(empty), Err(ProgramError::Rejected)));
}

#[test]
fn known_tweak_can_authorize_a_related_key_distinct_from_physical_internal_key() {
    let mut base = request(
        template_authorization_wasm_instance(TemplateKey::KnownTweak),
        false,
        None,
    );
    let secp = Secp256k1::new();
    let program = program_keypair(&base.instance);
    let (internal, parity) = program.x_only_public_key();
    let mut normalized = program.secret_key();
    if parity == Parity::Odd {
        normalized = normalized.negate();
    }
    let tap_tweak = TapTweakHash::from_key_and_tweak(internal, None).to_scalar();
    let mut delta = SecretKey::from_slice(&[1; 32]).unwrap();
    let (untweaked, tweak) = loop {
        let candidate = normalized.add_tweak(&Scalar::from(delta.negate())).unwrap();
        let candidate = KeyPair::from_secret_key(&secp, &candidate);
        if candidate.x_only_public_key().1 == Parity::Even {
            let tweak = SecretKey::from_slice(&tap_tweak.to_be_bytes())
                .unwrap()
                .add_tweak(&Scalar::from(delta))
                .unwrap();
            break (candidate, Scalar::from(tweak));
        }
        delta = delta.add_tweak(&Scalar::ONE).unwrap();
    };
    let (_, output_parity) = internal.tap_tweak(&secp, None);
    let related_key = untweaked.x_only_public_key().0;
    assert_ne!(related_key, internal);
    base.witness = known_tweak_witness(
        related_key,
        tweak,
        output_parity,
        Some(&signature(&base, &untweaked)),
    );
    oracle().sign(base).unwrap();
}

#[test]
fn registered_v2_template_authorization_executes_the_same_guest() {
    let evaluator =
        WasmEvaluator::with_version(WasmVersion::V2, TEMPLATE_AUTHORIZATION_WASM.to_vec()).unwrap();
    let mut base = request(
        ProgramInstance::new(evaluator.id(), vec![], vec![1]).unwrap(),
        true,
        None,
    );
    base.witness = signature(&base, &owner()).as_ref().to_vec();
    ProgramOracle::new(root(), vec![evaluator])
        .unwrap()
        .sign(base)
        .unwrap();
}

#[test]
fn templatehash_wasm_runs_every_official_bip446_case() {
    use bitcoin::consensus::deserialize;
    use bitcoin::hashes::hex::FromHex;
    use bitcoin::util::taproot::ControlBlock;
    use sapio_base::fragments::TEMPLATEHASH_WASM;
    #[derive(Deserialize)]
    struct Vector {
        spent_outputs: Vec<String>,
        spending_tx: String,
        input_index: u32,
        valid: bool,
        comment: String,
    }
    let vectors: Vec<Vector> = serde_json::from_str(include_str!(
        "../../../sapio-base/tests/data/bip446-basics.json"
    ))
    .unwrap();
    assert_eq!(vectors.len(), 19);
    for vector in vectors {
        let transaction: Transaction =
            deserialize(&Vec::<u8>::from_hex(&vector.spending_tx).unwrap()).unwrap();
        let outputs: Vec<TxOut> = vector
            .spent_outputs
            .iter()
            .map(|output| deserialize(&Vec::<u8>::from_hex(output).unwrap()).unwrap())
            .collect();
        let prevouts: Vec<_> = outputs.iter().collect();
        let mut script_witness = transaction.input[vector.input_index as usize].witness.to_vec();
        if script_witness.last().unwrap().first() == Some(&0x50) {
            script_witness.pop();
        }
        let control = ControlBlock::from_slice(script_witness.last().unwrap()).unwrap();
        let script = &script_witness[script_witness.len() - 2];
        assert_eq!(script.len(), 35);
        assert_eq!(script[0], 0x20);
        assert_eq!(&script[33..], &[0xce, 0x87]);
        let witness = &transaction.input[vector.input_index as usize].witness;
        let annex = if witness.len() >= 2 {
            witness.last().filter(|item| item.first() == Some(&0x50))
        } else {
            None
        };
        let view = SignedTransactionView {
            transaction: &transaction,
            prevouts: &prevouts,
            input_index: vector.input_index,
            internal_key: control.internal_key,
            annex,
        };
        let result = wasm::evaluate(
            WasmVersion::V2,
            TEMPLATEHASH_WASM,
            &[],
            &script[1..33],
            &view,
            &[],
        );
        assert_eq!(result.unwrap(), vector.valid, "{}", vector.comment);
    }
}
