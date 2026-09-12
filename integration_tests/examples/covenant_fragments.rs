//! Compile, authorize and finalize both fragment examples without broadcasting.

use bitcoin::consensus::encode::serialize_hex;
use bitcoin::secp256k1::Secp256k1;
use emulator_connect::program::ProgramOracle;
use sapio_contrib::contracts::template_authorization::Authorization;
use sapio_integration_tests::fragment_example::{
    compile_candidates, example_contract, participant_key, signing_request,
};
use sapio_integration_tests::program_example::{bind_candidates, example_root};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let secp = Secp256k1::new();
    let root = example_root();
    let oracle = ProgramOracle::new(root, vec![])?;
    let participant = participant_key();
    let mut examples = vec![];
    for (name, mode, authorizer) in [
        (
            "physical_internal_key",
            Authorization::InternalKey(participant.x_only_public_key().0),
            participant,
        ),
        (
            "known_tweak",
            Authorization::KnownTweak,
            root.to_keypair(&secp),
        ),
    ] {
        let source = example_contract(mode, oracle.public_root());
        let baseline = compile_candidates(&source, &[])?;
        let compiled = compile_candidates(&source, &[7_000])?;
        assert_eq!(baseline.address, compiled.address);
        assert_eq!(baseline.descriptor, compiled.descriptor);
        let mut spends = vec![];
        for candidate in bind_candidates(&compiled)? {
            let request = signing_request(
                &compiled,
                mode,
                candidate,
                &authorizer,
                Some(b"\x50sapio-fragments".to_vec()),
            )?;
            let evidence = request.witness.clone();
            let signed = oracle.sign(request)?;
            let finalized = sapio_psbt::finalize::finalize(signed, &secp)
                .map_err(|(_, errors)| format!("fragment finalization failed: {errors:?}"))?;
            let tx = finalized.extract_tx()?;
            assert_eq!(
                tx.input[0].witness.last(),
                Some(&b"\x50sapio-fragments"[..])
            );
            spends.push(serde_json::json!({
                "transaction": serialize_hex(&tx),
                "txid": tx.compute_txid(),
                "off_chain_authorization": evidence,
            }));
        }
        examples.push(serde_json::json!({"mode": name, "artifact": compiled, "spends": spends}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "research_only": true,
            "funding": "synthetic; no transactions broadcast",
            "examples": examples,
        }))?
    );
    Ok(())
}
