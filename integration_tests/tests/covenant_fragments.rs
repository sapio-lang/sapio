use bitcoin::secp256k1::Secp256k1;
use bitcoin::{Amount, Network, TxOut};
use emulator_connect::program::{ProgramError, ProgramOracle, ProgramSpendPath};
use sapio::contract::{Compilable, CompilationError, Context};
use sapio_base::effects::EffectPath;
use sapio_contrib::contracts::template_authorization::{Authorization, FragmentContract};
use sapio_integration_tests::fragment_example::{
    compile_candidates, example_contract, participant_key, signing_request,
};
use sapio_integration_tests::program_example::{bind_candidates, example_root, recipient};
use std::sync::Arc;

#[test]
fn contract_uses_public_terms_and_supplied_funds_without_default_proposals() {
    let oracle = ProgramOracle::new(example_root(), vec![]).unwrap();
    let destination = recipient(97);
    let change = recipient(98);
    let source = FragmentContract::new(
        Authorization::KnownTweak,
        oracle.public_root(),
        destination.clone(),
        change.clone(),
        Amount::from_sat(1_000),
    )
    .unwrap();
    let context = |amount: Option<u64>| {
        let candidates: std::collections::BTreeMap<_, _> = amount
            .map(|amount| ("payment", amount))
            .into_iter()
            .collect();
        let effects = serde_json::from_value(serde_json::json!({
            "effects": {"fragments/@action/pay/@suggested": candidates}
        }))
        .unwrap();
        Context::new(
            Network::Regtest,
            Amount::from_sat(50_000),
            sapio_base::LoweringPlan::Native,
            EffectPath::try_from("fragments").unwrap(),
            Arc::new(effects),
            None,
        )
    };
    let baseline = source.compile(context(None)).unwrap();
    assert!(baseline.suggested_txs.is_empty());
    let compiled = source.compile(context(Some(45_000))).unwrap();
    assert_eq!(baseline.address, compiled.address);
    assert_eq!(compiled.suggested_txs.len(), 1);
    let template = compiled.suggested_txs.values().next().unwrap();
    assert_eq!(
        template.tx.output,
        [
            TxOut {
                value: Amount::from_sat(45_000),
                script_pubkey: destination.script_pubkey(),
            },
            TxOut {
                value: Amount::from_sat(4_000),
                script_pubkey: change.script_pubkey(),
            },
        ]
    );
    assert_eq!(template.required_input_amount, Amount::from_sat(50_000));
    assert!(matches!(
        source.compile(context(Some(49_001))),
        Err(CompilationError::OutOfFunds)
    ));
}

#[test]
fn compiled_fragment_contracts_authorize_both_paths_and_preserve_annexes() {
    let secp = Secp256k1::new();
    let root = example_root();
    let oracle = ProgramOracle::new(root, vec![]).unwrap();
    let participant = participant_key();
    for (mode, signer, script_path) in [
        (
            Authorization::InternalKey(participant.x_only_public_key().0),
            participant,
            true,
        ),
        (Authorization::KnownTweak, root.to_keypair(&secp), false),
    ] {
        let source = example_contract(mode, oracle.public_root());
        let baseline = compile_candidates(&source, &[]).unwrap();
        let compiled = compile_candidates(&source, &[7_000]).unwrap();
        assert_eq!(baseline.address, compiled.address);
        assert_eq!(baseline.descriptor, compiled.descriptor);
        let candidates = bind_candidates(&compiled).unwrap();
        assert_eq!(candidates.len(), 2);
        for candidate in candidates {
            if script_path {
                assert_eq!(
                    candidate.inputs[0].tap_internal_key,
                    Some(participant.x_only_public_key().0)
                );
            }
            let request = signing_request(
                &source,
                candidate,
                &signer,
                Some(b"\x50fragments-test".to_vec()),
            )
            .unwrap();
            assert_eq!(
                matches!(request.path, ProgramSpendPath::ScriptPath(_)),
                script_path
            );
            if !script_path {
                assert!(request.psbt.0.inputs[0].tap_internal_key.is_none());
                assert!(request.psbt.0.inputs[0].tap_scripts.is_empty());
            }
            let mut bad_evidence = request.clone();
            *bad_evidence.witness.last_mut().unwrap() ^= 1;
            assert!(matches!(
                oracle.sign(bad_evidence),
                Err(ProgramError::Evaluation(_))
            ));
            let mut bad_annex = request.clone();
            sapio_psbt::annex::set(
                &mut bad_annex.psbt.0.inputs[0],
                Some(b"\x50other-annex".to_vec()),
            )
            .unwrap();
            assert!(matches!(
                oracle.sign(bad_annex),
                Err(ProgramError::Evaluation(_))
            ));
            let signed = oracle.sign(request).unwrap();
            let finalized = sapio_psbt::finalize::finalize(signed, &secp).unwrap();
            let tx = finalized.extract_tx().unwrap();
            assert_eq!(tx.input[0].witness.last(), Some(&b"\x50fragments-test"[..]));
            assert_eq!(tx.input[0].witness.len(), if script_path { 4 } else { 2 });
        }
    }
}
