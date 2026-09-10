//! Export explicitly signed custom-policy spends for a Bitcoin Core oracle.
//!
//! Without arguments, print funding addresses and placeholder-outpoint vectors.
//! With a JSON file mapping group names to funded OutPoints, sign those actual
//! spends. `contrib/check_custom_policy.py` drives both stages on isolated regtest.

#[path = "../tests/fixtures/custom_policy.rs"]
mod fixture;

use bitcoin::consensus::encode::serialize_hex;
use bitcoin::hashes::hex::ToHex;
use bitcoin::{Address, Network, OutPoint};
use fixture::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn case(name: &str, allowed: bool, transaction: bitcoin::Transaction) -> Value {
    json!({"name": name, "allowed": allowed, "transaction": serialize_hex(&transaction)})
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let funding: BTreeMap<String, OutPoint> = match std::env::args_os().nth(1) {
        Some(path) => serde_json::from_slice(&std::fs::read(path)?)?,
        None => BTreeMap::new(),
    };
    let mut groups = vec![];
    for (name, protected) in [("pure_custom", false), ("composed_emulated", true)] {
        // These oracle cases use signer emulation: unmodified Bitcoin Core
        // does not implement native CheckTemplateVerify consensus semantics.
        let compiled = compiled(protected, true);
        let raw = tree(&compiled);
        let outpoint = funding.get(name).copied().unwrap_or_default();
        let unsigned = unsigned_spend(&compiled, outpoint);
        let valid = signed_spend(&compiled, unsigned.clone(), None, false);
        let mut cases = vec![case("valid", true, valid.clone())];
        for owner in 1..=if protected { 4 } else { 1 } {
            cases.push(case(
                &format!("missing_signer_{owner}"),
                false,
                signed_spend(&compiled, unsigned.clone(), Some(owner), false),
            ));
        }
        cases.push(case(
            "wrong_owner_signature",
            false,
            signed_spend(&compiled, unsigned.clone(), None, true),
        ));
        let mut tampered = valid;
        tampered.output[0].value -= 1;
        cases.push(case("output_changed_after_signing", false, tampered));
        if protected {
            let mut changed = unsigned;
            changed.output[0].value -= 1;
            cases.push(case(
                "changed_output_without_covenant_signer",
                false,
                signed_spend(&compiled, changed, Some(4), false),
            ));
        }
        // Keep the original two accepted and ten rejected cases mandatory
        // even though the Core driver also accepts other policy vector sets.
        assert_eq!(
            cases.iter().filter(|case| case["allowed"] == true).count(),
            1
        );
        assert_eq!(cases.len(), if protected { 8 } else { 4 });
        groups.push(json!({
            "name": name,
            "address": Address::p2tr_tweaked(raw.spend_info().output_key(), Network::Regtest).to_string(),
            "funding_amount_sats": 10_000,
            "script": raw.leaves()[0].1.as_bytes().to_hex(),
            "cases": cases,
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"groups": groups}))?
    );
    Ok(())
}
