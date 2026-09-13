//! Export program-emulated spends for an independent Bitcoin Core check.
//!
//! `contrib/check_custom_policy.py` obtains the funding address, funds it on an
//! isolated regtest chain, then calls this executable with the actual outpoint.

use bitcoin::blockdata::opcodes::all::OP_CHECKSIG;
use bitcoin::blockdata::script::Builder;
use bitcoin::consensus::encode::serialize_hex;
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::taproot::{LeafVersion, TapLeafHash, TaprootBuilder};
use bitcoin::{Address, Network, OutPoint, ScriptBuf, Transaction, TxIn, TxOut};
use emulator_connect::program::{ProgramOracle, ProgramSigningRequest, ProgramSpendPath, PSBT};
use miniscript::psbt::PsbtExt;
use sapio_base::program::ctv_wasm_instance;
use sapio_base::{CTVHash, Ctv};
use sapio_integration_tests::program_example::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::error::Error;

fn case(name: String, allowed: bool, transaction: bitcoin::Transaction) -> Value {
    json!({"name": name, "allowed": allowed, "transaction": serialize_hex(&transaction)})
}

fn main() -> Result<(), Box<dyn Error>> {
    let funding: BTreeMap<String, OutPoint> = match std::env::args_os().nth(1) {
        Some(path) => serde_json::from_slice(&std::fs::read(path)?)?,
        None => BTreeMap::new(),
    };
    let oracle = ProgramOracle::new(example_root(), vec![pay_at_least_evaluator()])?;
    let destination = recipient(92);
    let contract = PaymentContract::new(5_000, destination.clone(), oracle.public_root());
    let compiled = contract.compile_candidates(&[
        PaymentCandidate {
            amount: 6_000,
            recipient_first: true,
        },
        PaymentCandidate {
            amount: 7_000,
            recipient_first: false,
        },
    ])?;
    let address = Address::from_script(
        &bitcoin::ScriptBuf::from(&compiled.address),
        Network::Regtest,
    )?;
    let name = "program_pay_at_least";
    let outpoint = funding.get(name).copied().unwrap_or_default();
    let mut cases = vec![];
    for mut candidate in bind_candidates(&compiled)? {
        let index = candidate
            .unsigned_tx
            .output
            .iter()
            .position(|output| output.script_pubkey == destination.script_pubkey())
            .ok_or("candidate does not pay the fixed recipient")?;
        let amount = candidate.unsigned_tx.output[index].value.to_sat();
        if amount == 5_000 {
            continue;
        }
        // Binding populated the exact funding output. Replace its synthetic
        // parent with the driver's real outpoint, retaining the witness UTXO
        // that Core will check against that output during signature validation.
        assert_eq!(candidate.inputs.len(), 1);
        assert_eq!(
            candidate.inputs[0]
                .witness_utxo
                .as_ref()
                .unwrap()
                .value
                .to_sat(),
            FUNDING_SATS,
        );
        candidate.inputs[0].non_witness_utxo = None;
        candidate.unsigned_tx.input[0].previous_output = outpoint;
        let mut signed = oracle.sign(signing_request(&compiled, candidate, index as u32)?)?;
        signed
            .finalize_mut(&Secp256k1::new())
            .map_err(|errors| format!("program spend failed finalization: {errors:?}"))?;
        sapio_integration_tests::program_example::check_finalized_candidate(&compiled, &signed)?;
        let transaction = signed.extract_tx()?;
        let label = format!("pay_{amount}_at_{index}");
        cases.push(case(label.clone(), true, transaction.clone()));
        let mut amount_changed = transaction.clone();
        amount_changed.output[index].value -= bitcoin::Amount::ONE_SAT;
        cases.push(case(
            format!("{label}_amount_changed"),
            false,
            amount_changed,
        ));
        let mut recipient_changed = transaction;
        recipient_changed.output[index].script_pubkey = recipient(93).script_pubkey();
        cases.push(case(
            format!("{label}_recipient_changed"),
            false,
            recipient_changed,
        ));
    }
    assert_eq!(cases.len(), 6);
    assert_eq!(
        cases.iter().filter(|case| case["allowed"] == true).count(),
        2
    );
    let key_path_group = json!({
        "name": name,
        "address": address.to_string(),
        "funding_amount_sats": FUNDING_SATS,
        "cases": cases,
    });

    // Independently exercise BIP341's tapscript extension as well as key-path
    // signatures. The predicate key is in a real, authenticated CHECKSIG leaf.
    let name = "program_pay_at_least_scriptpath";
    let secp = Secp256k1::new();
    let instance = instance(5_000, &destination.script_pubkey());
    let program_key = instance.derive_public_key(&oracle.public_root())?;
    let script = Builder::new()
        .push_slice(&program_key.serialize())
        .push_opcode(OP_CHECKSIG)
        .into_script();
    let internal = example_root().to_keypair(&secp).x_only_public_key().0;
    let spend = TaprootBuilder::new()
        .add_leaf(0, script.clone())?
        .finalize(&secp, internal)
        .expect("single-leaf example tree");
    let script_pubkey = ScriptBuf::new_p2tr_tweaked(spend.output_key());
    let mut psbt = Psbt::from_unsigned_tx(Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::absolute::LockTime::from_consensus(0),
        input: vec![TxIn {
            previous_output: funding.get(name).copied().unwrap_or_default(),
            sequence: bitcoin::Sequence(0xffff_fffd),
            ..TxIn::default()
        }],
        output: vec![
            TxOut {
                value: bitcoin::Amount::from_sat(6_000),
                script_pubkey: destination.script_pubkey(),
            },
            TxOut {
                value: bitcoin::Amount::from_sat(13_500),
                script_pubkey: recipient(94).script_pubkey(),
            },
        ],
    })?;
    let leaf = (script, LeafVersion::TapScript);
    let leaf_hash = TapLeafHash::from_script(&leaf.0, leaf.1);
    let input = &mut psbt.inputs[0];
    input.witness_utxo = Some(TxOut {
        value: bitcoin::Amount::from_sat(FUNDING_SATS),
        script_pubkey,
    });
    input.tap_internal_key = Some(internal);
    input.tap_merkle_root = spend.merkle_root();
    input
        .tap_scripts
        .insert(spend.control_block(&leaf).unwrap(), leaf);
    let mut signed = oracle.sign(ProgramSigningRequest {
        instance,
        input_index: 0,
        witness: 0_u32.to_le_bytes().to_vec(),
        path: ProgramSpendPath::ScriptPath(leaf_hash),
        psbt: PSBT(psbt),
    })?;
    signed
        .finalize_mut(&secp)
        .map_err(|errors| format!("program script-path spend failed finalization: {errors:?}"))?;
    let transaction = signed.extract_tx()?;
    assert_eq!(transaction.input[0].witness.len(), 3);
    let mut changed = transaction.clone();
    changed.output[0].value -= bitcoin::Amount::ONE_SAT;
    let script_path_group = json!({
        "name": name,
        "address": Address::p2tr_tweaked(spend.output_key(), Network::Regtest).to_string(),
        "funding_amount_sats": FUNDING_SATS,
        "cases": [
            case("valid".into(), true, transaction),
            case("amount_changed_after_signing".into(), false, changed),
        ],
    });

    // The inline evaluator commits the exact transaction template. Its hash
    // does not depend on the funding outpoint or the derived program key.
    let name = "program_inline_ctv";
    let mut transaction = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::absolute::LockTime::from_consensus(0),
        input: vec![TxIn {
            sequence: bitcoin::Sequence(0xffff_fffd),
            ..TxIn::default()
        }],
        output: vec![TxOut {
            value: bitcoin::Amount::from_sat(FUNDING_SATS - 500),
            script_pubkey: recipient(95).script_pubkey(),
        }],
    };
    let instance = ctv_wasm_instance(Ctv(transaction.get_ctv_hash(0)));
    let key = instance.derive_public_key(&oracle.public_root())?;
    let address = Address::p2tr(&secp, key, None, Network::Regtest);
    transaction.input[0].previous_output = funding.get(name).copied().unwrap_or_default();
    let mut psbt = Psbt::from_unsigned_tx(transaction)?;
    psbt.inputs[0].tap_internal_key = Some(key);
    psbt.inputs[0].witness_utxo = Some(TxOut {
        value: bitcoin::Amount::from_sat(FUNDING_SATS),
        script_pubkey: address.script_pubkey(),
    });
    let mut signed = oracle.sign(ProgramSigningRequest {
        instance,
        input_index: 0,
        witness: vec![],
        path: ProgramSpendPath::KeyPath,
        psbt: PSBT(psbt),
    })?;
    signed
        .finalize_mut(&secp)
        .map_err(|errors| format!("inline CTV spend failed finalization: {errors:?}"))?;
    let transaction = signed.extract_tx()?;
    assert_eq!(transaction.input[0].witness.len(), 1);
    let mut amount_changed = transaction.clone();
    amount_changed.output[0].value -= bitcoin::Amount::ONE_SAT;
    let mut sequence_changed = transaction.clone();
    sequence_changed.input[0].sequence =
        bitcoin::Sequence(sequence_changed.input[0].sequence.to_consensus_u32() - 1);
    let ctv_group = json!({
        "name": name,
        "address": address.to_string(),
        "funding_amount_sats": FUNDING_SATS,
        "cases": [
            case("valid".into(), true, transaction),
            case("amount_changed_after_signing".into(), false, amount_changed),
            case("sequence_changed_after_signing".into(), false, sequence_changed),
        ],
    });
    let groups = [key_path_group, script_path_group, ctv_group];
    assert_eq!(
        groups
            .iter()
            .map(|group| group["cases"].as_array().unwrap().len())
            .sum::<usize>(),
        11
    );
    assert_eq!(
        groups
            .iter()
            .flat_map(|group| group["cases"].as_array().unwrap())
            .filter(|case| case["allowed"] == true)
            .count(),
        4
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"groups": groups}))?
    );
    Ok(())
}
