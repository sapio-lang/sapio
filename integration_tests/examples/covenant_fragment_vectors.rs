//! Emit annex-bearing fragment spends for the isolated Core test driver.

use bitcoin::consensus::encode::serialize_hex;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{Address, Network, OutPoint, Transaction, Witness};
use emulator_connect::program::ProgramOracle;
use sapio_integration_tests::fragment_example::{participant_key, Authorization, FragmentContract};
use sapio_integration_tests::program_example::{bind_candidates, example_root, FUNDING_SATS};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::error::Error;

fn case(name: &str, allowed: bool, transaction: Transaction) -> Value {
    json!({"name": name, "allowed": allowed, "transaction": serialize_hex(&transaction)})
}

fn main() -> Result<(), Box<dyn Error>> {
    let funding: BTreeMap<String, OutPoint> = match std::env::args_os().nth(1) {
        Some(path) => serde_json::from_slice(&std::fs::read(path)?)?,
        None => BTreeMap::new(),
    };
    let secp = Secp256k1::new();
    let root = example_root();
    let oracle = ProgramOracle::new(root, vec![])?;
    let participant = participant_key();
    let mut groups = vec![];
    for (name, mode, authorizer) in [
        (
            "fragment_internal_key",
            Authorization::InternalKey(participant.x_only_public_key().0),
            participant,
        ),
        (
            "fragment_known_tweak",
            Authorization::KnownTweak,
            root.to_keypair(&secp),
        ),
    ] {
        let source = FragmentContract::new(mode, oracle.public_root());
        let compiled = source.compile_candidates(&[])?;
        let mut candidates = bind_candidates(&compiled)?;
        assert_eq!(candidates.len(), 1);
        let mut candidate = candidates.remove(0);
        candidate.unsigned_tx.input[0].previous_output =
            funding.get(name).copied().unwrap_or_default();
        candidate.inputs[0].non_witness_utxo = None;
        let request = source.signing_request(
            candidate,
            &authorizer,
            Some(b"\x50sapio-fragments".to_vec()),
        )?;
        let signed = oracle.sign(request)?;
        let finalized = sapio_psbt::finalize::finalize(signed, &secp)
            .map_err(|(_, errors)| format!("fragment vector finalization failed: {errors:?}"))?;
        let transaction = finalized.extract_tx()?;
        let mut amount_changed = transaction.clone();
        amount_changed.output[0].value -= bitcoin::Amount::ONE_SAT;
        let mut annex_changed = transaction.clone();
        let mut witness = transaction.input[0].witness.to_vec();
        witness.last_mut().ok_or("missing annex witness")?[1] ^= 1;
        annex_changed.input[0].witness = Witness::from_slice(&witness);
        let mut annex_removed = transaction.clone();
        let mut witness = transaction.input[0].witness.to_vec();
        assert_eq!(witness.pop(), Some(b"\x50sapio-fragments".to_vec()));
        annex_removed.input[0].witness = Witness::from_slice(&witness);
        let address = Address::from_script(
            &bitcoin::ScriptBuf::from(&compiled.address),
            Network::Regtest,
        )?;
        groups.push(json!({
            "name": name,
            "address": address.to_string(),
            "funding_amount_sats": FUNDING_SATS,
            "cases": [
                case("valid", true, transaction),
                case("amount_changed_after_signing", false, amount_changed),
                case("annex_changed_after_signing", false, annex_changed),
                case("annex_removed_after_signing", false, annex_removed),
            ],
        }));
    }
    assert_eq!(groups.len(), 2);
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"groups": groups}))?
    );
    Ok(())
}
