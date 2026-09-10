//! Print a research artifact and locally validated spends; nothing is broadcast.

use bitcoin::consensus::encode::serialize_hex;
use bitcoin::secp256k1::Secp256k1;
use emulator_connect::program::ProgramOracle;
use miniscript::psbt::PsbtExt;
use sapio_integration_tests::program_example::*;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let oracle = ProgramOracle::new(example_root(), vec![pay_at_least_evaluator()])?;
    let recipient = recipient(92);
    let contract = PaymentContract::new(5_000, recipient.clone(), oracle.public_root());
    let original = contract.compile_candidates(&[])?;
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
    assert_eq!(original.address, compiled.address);
    let mut transactions = Vec::new();
    for candidate in bind_candidates(&compiled)? {
        let output_index = candidate
            .unsigned_tx
            .output
            .iter()
            .position(|output| output.script_pubkey == recipient.script_pubkey())
            .expect("candidate pays the fixed recipient") as u32;
        let mut signed = oracle.sign(signing_request(
            contract.emulation(),
            candidate,
            output_index,
        ))?;
        signed
            .finalize_mut(&Secp256k1::new())
            .map_err(|errors| format!("sample spend failed finalization: {errors:?}"))?;
        let transaction = signed.extract_tx();
        transactions.push(serde_json::json!({
            "output_witness": output_index,
            "transaction": serialize_hex(&transaction),
            "txid": transaction.txid(),
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "research_only": true,
            "funding": "synthetic; no transactions broadcast",
            "artifact": compiled,
            "validated_spends": transactions,
        }))?
    );
    Ok(())
}
