use super::*;
use emulator_connect::program::completion::SpendIntent;
use emulator_connect::program::spend_plan::{BranchStatus, SpendRequirement};
use emulator_connect::program::{
    plan_spends, prepare_spend, ArtifactProgramError, ProgramError, ProgramOracle, SpendPath,
    SpendPlanError, WasmEvaluator,
};

fn prepare(demo: &Demo) -> Result<SpendIntent, SpendPlanError> {
    prepare_spend(
        &demo.artifact,
        SpendPath::KeyPath,
        demo.psbt.clone(),
        0,
        &demo.assets,
        &demo.evidence,
    )
    .map(SpendIntent::from_prepared)
}

fn oracle(demo: &Demo) -> ProgramOracle {
    ProgramOracle::new(
        demo.oracle,
        vec![WasmEvaluator::new(EVALUATOR_WASM.to_vec()).unwrap()],
    )
    .unwrap()
}

#[test]
fn payment_resumes_from_public_files_and_completes_one_program_branch() {
    let demo = demo(6_000).unwrap();
    let report = plan_spends(
        &demo.artifact,
        Some((&demo.psbt, 0)),
        &SpendAssets::default(),
    )
    .unwrap();
    assert!(!report.branches.is_empty());
    for branch in &report.branches {
        assert_eq!(branch.status, BranchStatus::MissingAssets);
        assert!(branch
            .requirements
            .iter()
            .any(|requirement| matches!(requirement, SpendRequirement::Program { .. })));
    }
    let intent = prepare(&demo).unwrap();
    assert_eq!(intent.requests().len(), 1);
    let response = oracle(&demo).sign(intent.requests()[0].clone()).unwrap();
    let artifact: Compiled =
        serde_json::from_slice(&serde_json::to_vec(&demo.artifact).unwrap()).unwrap();
    let intent: SpendIntent =
        serde_json::from_slice(&serde_json::to_vec(&intent).unwrap()).unwrap();
    let mut current = Psbt::deserialize(&intent.baseline_psbt().serialize()).unwrap();
    intent
        .merge_response(&artifact, &mut current, 0, &response)
        .unwrap();
    let finalized = intent
        .finalize(&artifact, &current, &Secp256k1::new())
        .unwrap();
    assert_eq!(finalized.fee().unwrap().to_sat(), 500);
    let transaction = finalized.extract_tx().unwrap();
    assert_eq!(transaction.output[0].value.to_sat(), 6_000);
    assert_eq!(
        transaction.output[0].script_pubkey,
        destination(92).unwrap().script_pubkey()
    );
    assert_eq!(transaction.output[1].value.to_sat(), 13_500);
    assert!(!transaction.input[0].witness.is_empty());
}

#[test]
fn proposing_underpayment_does_not_change_policy_or_authorize_it() {
    let allowed = demo(MINIMUM_SATS).unwrap();
    let allowed_intent = prepare(&allowed).unwrap();
    oracle(&allowed)
        .sign(allowed_intent.requests()[0].clone())
        .unwrap();
    let below_minimum = demo(MINIMUM_SATS - 1).unwrap();
    assert_eq!(allowed.artifact.address, below_minimum.artifact.address);
    let intent = prepare(&below_minimum).unwrap();
    assert!(matches!(
        oracle(&below_minimum).sign(intent.requests()[0].clone()),
        Err(ProgramError::Rejected)
    ));
}

#[test]
fn extra_funding_cannot_silently_increase_the_retained_fee() {
    let mut demo = demo(6_000).unwrap();
    let funding = demo.psbt.inputs[0].non_witness_utxo.as_mut().unwrap();
    funding.output[0].value += Amount::ONE_SAT;
    let outpoint = OutPoint::new(funding.compute_txid(), 0);
    let output = funding.output[0].clone();
    demo.psbt.unsigned_tx.input[0].previous_output = outpoint;
    demo.psbt.inputs[0].witness_utxo = Some(output);
    assert!(matches!(
        prepare(&demo),
        Err(SpendPlanError::Artifact(ArtifactProgramError::Funding(_)))
    ));
}
