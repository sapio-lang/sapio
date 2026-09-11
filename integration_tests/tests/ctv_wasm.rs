//! Execute the checked-in Rust guest through the actual program oracle.

use bitcoin::bip32::Xpub;
use bitcoin::consensus::deserialize;
use bitcoin::hex::FromHex;
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{Address, Network, ScriptBuf, Transaction, TxIn, TxOut};
use emulator_connect::program::{
    ProgramError, ProgramOracle, ProgramSigningRequest, ProgramSpendPath, WasmEvaluator, PSBT,
};
use sapio_base::program::{ctv_wasm_instance, ProgramInstance};
use sapio_base::{CTVHash, Ctv};
use sapio_integration_tests::program_example::{
    evaluator_id, example_root, parameters, pay_at_least_evaluator, recipient, PAY_AT_LEAST,
};
use serde::Deserialize;

fn request(
    instance: ProgramInstance,
    mut transaction: Transaction,
    index: u32,
) -> ProgramSigningRequest {
    for input in &mut transaction.input {
        input.script_sig = ScriptBuf::new();
        input.witness = Default::default();
    }
    let root = Xpub::from_priv(&Secp256k1::new(), &example_root());
    let key = instance.derive_public_key(&root).unwrap();
    let mut psbt = Psbt::from_unsigned_tx(transaction).unwrap();
    for input in &mut psbt.inputs {
        input.witness_utxo = Some(TxOut {
            value: bitcoin::Amount::from_sat(100_000),
            script_pubkey: recipient(92).script_pubkey(),
        });
    }
    let input = &mut psbt.inputs[index as usize];
    input.tap_internal_key = Some(key);
    input.witness_utxo.as_mut().unwrap().script_pubkey =
        Address::p2tr(&Secp256k1::new(), key, None, Network::Regtest).script_pubkey();
    ProgramSigningRequest {
        instance,
        input_index: index,
        witness: vec![],
        path: ProgramSpendPath::KeyPath,
        psbt: PSBT(psbt),
    }
}

fn transaction() -> Transaction {
    Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::absolute::LockTime::from_consensus(123),
        input: vec![
            TxIn {
                previous_output: bitcoin::OutPoint {
                    vout: 0,
                    ..Default::default()
                },
                sequence: bitcoin::Sequence(20),
                ..TxIn::default()
            },
            TxIn {
                previous_output: bitcoin::OutPoint {
                    vout: 1,
                    ..Default::default()
                },
                sequence: bitcoin::Sequence(30),
                ..TxIn::default()
            },
        ],
        output: vec![
            TxOut {
                value: bitcoin::Amount::from_sat(50_000),
                script_pubkey: recipient(92).script_pubkey(),
            },
            TxOut {
                value: bitcoin::Amount::from_sat(70_000),
                script_pubkey: recipient(93).script_pubkey(),
            },
        ],
    }
}

#[derive(Deserialize)]
struct Vector {
    hex_tx: String,
    spend_index: Vec<u32>,
    result: Vec<String>,
}

#[test]
fn compiled_ctv_guest_matches_all_applicable_official_bip119_vectors() {
    let entries: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../sapio-base/tests/data/ctvhash.json")).unwrap();
    let oracle = ProgramOracle::new(example_root(), vec![]).unwrap();
    let mut hashes = 0;
    for entry in entries.into_iter().filter(|entry| !entry.is_string()) {
        let vector: Vector = serde_json::from_value(entry).unwrap();
        let tx: Transaction = deserialize(&Vec::<u8>::from_hex(&vector.hex_tx).unwrap()).unwrap();
        // The signed view cannot carry arbitrary scriptSig bytes. The program
        // deliberately covers native witness prevouts and valid input indices.
        if tx.input.iter().any(|input| !input.script_sig.is_empty()) {
            continue;
        }
        for (index, expected) in vector.spend_index.into_iter().zip(vector.result) {
            if index as usize >= tx.input.len() {
                continue;
            }
            let instance = ctv_wasm_instance(Ctv(expected.parse().unwrap()));
            let signed = oracle
                .sign(request(instance, tx.clone(), index))
                .unwrap_or_else(|error| {
                    panic!("CTV guest rejected official hash {expected} at input {index}: {error}")
                });
            assert!(signed.inputs[index as usize].tap_key_sig.is_some());
            hashes += 1;
        }
    }
    assert_eq!(hashes, 96);
}

#[test]
fn ctv_guest_commits_every_ctv_field_and_the_selected_input() {
    let oracle = ProgramOracle::new(example_root(), vec![]).unwrap();
    let tx = transaction();
    let instance = ctv_wasm_instance(Ctv(tx.get_ctv_hash(0)));
    oracle
        .sign(request(instance.clone(), tx.clone(), 0))
        .unwrap();
    let mut mutations = vec![];
    let mut changed = tx.clone();
    changed.version.0 += 1;
    mutations.push(changed);
    let mut changed = tx.clone();
    changed.lock_time =
        bitcoin::absolute::LockTime::from_consensus(changed.lock_time.to_consensus_u32() + 1);
    mutations.push(changed);
    for index in 0..tx.input.len() {
        let mut changed = tx.clone();
        changed.input[index].sequence =
            bitcoin::Sequence(changed.input[index].sequence.to_consensus_u32() + 1);
        mutations.push(changed);
    }
    let mut changed = tx.clone();
    changed.input.push(TxIn::default());
    mutations.push(changed);
    let mut changed = tx.clone();
    changed.output[0].value -= bitcoin::Amount::ONE_SAT;
    mutations.push(changed);
    let mut changed = tx.clone();
    changed.output[0].script_pubkey = recipient(94).script_pubkey();
    mutations.push(changed);
    let mut changed = tx.clone();
    changed.output.push(tx.output[0].clone());
    mutations.push(changed);
    let mut changed = tx.clone();
    changed.output.swap(0, 1);
    mutations.push(changed);
    for changed in mutations {
        assert!(matches!(
            oracle.sign(request(instance.clone(), changed, 0)),
            Err(ProgramError::Rejected)
        ));
    }
    assert!(matches!(
        oracle.sign(request(instance, tx.clone(), 1)),
        Err(ProgramError::Rejected)
    ));
    oracle
        .sign(request(ctv_wasm_instance(Ctv(tx.get_ctv_hash(1))), tx, 1))
        .unwrap();
}

#[test]
fn ctv_guest_requires_native_witness_prevouts_but_does_not_commit_their_values() {
    let oracle = ProgramOracle::new(example_root(), vec![]).unwrap();
    let tx = transaction();
    let base = request(ctv_wasm_instance(Ctv(tx.get_ctv_hash(0))), tx, 0);
    for version in 0..=16 {
        for length in [2, 20, 32, 40] {
            let mut allowed = base.clone();
            let mut script = vec![if version == 0 { 0 } else { 0x50 + version }, length];
            script.resize(length as usize + 2, 42);
            let previous = allowed.psbt.0.inputs[1].witness_utxo.as_mut().unwrap();
            previous.script_pubkey = ScriptBuf::from(script);
            previous.value += bitcoin::Amount::ONE_SAT;
            allowed.psbt.0.unsigned_tx.input[1].previous_output.vout = 99;
            oracle.sign(allowed).unwrap();
        }
    }
    for script in [
        Vec::<u8>::from_hex("76a914000000000000000000000000000000000000000088ac").unwrap(),
        Vec::<u8>::from_hex("a914000000000000000000000000000000000000000087").unwrap(),
        vec![0, 1, 42],
        vec![0, 2, 42],
        vec![0, 2, 42, 42, 42],
        vec![0, 0x4c, 2, 42, 42],
        vec![0x61, 2, 42, 42],
    ] {
        let mut unsupported = base.clone();
        unsupported.psbt.0.inputs[1]
            .witness_utxo
            .as_mut()
            .unwrap()
            .script_pubkey = ScriptBuf::from(script);
        assert!(matches!(
            oracle.sign(unsupported),
            Err(ProgramError::Rejected)
        ));
    }
}

#[test]
fn ctv_guest_uses_consensus_compact_size_at_each_length_boundary() {
    let oracle = ProgramOracle::new(example_root(), vec![]).unwrap();
    for length in [0, 252, 253, 65_535, 65_536] {
        let mut tx = transaction();
        tx.output[0].script_pubkey = ScriptBuf::from(vec![42; length]);
        let instance = ctv_wasm_instance(Ctv(tx.get_ctv_hash(0)));
        oracle.sign(request(instance, tx, 0)).unwrap();
    }
}

#[test]
fn ctv_guest_rejects_parameter_and_evidence_extensions() {
    let oracle = ProgramOracle::new(example_root(), vec![]).unwrap();
    let tx = transaction();
    let instance = ctv_wasm_instance(Ctv(tx.get_ctv_hash(0)));
    for length in [0, 31, 33] {
        let malformed = ProgramInstance::new(
            instance.evaluator(),
            instance.program().to_vec(),
            vec![0; length],
        )
        .unwrap();
        assert!(matches!(
            oracle.sign(request(malformed, tx.clone(), 0)),
            Err(ProgramError::Evaluation(_))
        ));
    }
    let evaluator = WasmEvaluator::new(instance.program().to_vec()).unwrap();
    let extra_program = ProgramInstance::new(
        evaluator.id(),
        b"unexpected-selector".to_vec(),
        instance.parameters().to_vec(),
    )
    .unwrap();
    let registered = ProgramOracle::new(example_root(), vec![evaluator]).unwrap();
    assert!(matches!(
        registered.sign(request(extra_program, tx.clone(), 0)),
        Err(ProgramError::Evaluation(_))
    ));
    let mut evidence = request(instance, tx, 0);
    evidence.witness.push(0);
    assert!(matches!(
        oracle.sign(evidence),
        Err(ProgramError::Evaluation(_))
    ));
}

#[test]
fn compiled_payment_interpreter_consumes_parameters_and_selector_exactly() {
    let oracle = ProgramOracle::new(example_root(), vec![pay_at_least_evaluator()]).unwrap();
    let encoded = parameters(5_000, &recipient(92).script_pubkey());
    let mut malformed: Vec<_> = (0..encoded.len())
        .map(|length| encoded[..length].to_vec())
        .collect();
    let mut trailing = encoded.clone();
    trailing.push(0);
    malformed.push(trailing);
    for length in [0, u32::MAX] {
        let mut wrong_length = encoded.clone();
        wrong_length[8..12].copy_from_slice(&length.to_le_bytes());
        malformed.push(wrong_length);
    }
    for parameters in malformed {
        let instance =
            ProgramInstance::new(evaluator_id(), PAY_AT_LEAST.to_vec(), parameters).unwrap();
        let mut request = request(instance, transaction(), 0);
        request.witness = 0_u32.to_le_bytes().to_vec();
        assert!(matches!(
            oracle.sign(request),
            Err(ProgramError::Evaluation(_))
        ));
    }
    let instance =
        ProgramInstance::new(evaluator_id(), b"pay-at-least/v2".to_vec(), encoded).unwrap();
    let mut request = request(instance, transaction(), 0);
    request.witness = 0_u32.to_le_bytes().to_vec();
    assert!(matches!(
        oracle.sign(request),
        Err(ProgramError::Evaluation(_))
    ));
}
