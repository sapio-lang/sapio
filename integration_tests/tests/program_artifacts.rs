//! Exported contracts remain sufficient for signing in a separate process.

use bitcoin::bip32::Xpub;
use bitcoin::secp256k1::Secp256k1;
use emulator_connect::program::{
    prepare_program_request, validate_program_response, ProgramOracle, ProgramSigningRequest, PSBT,
};
use sapio::contract::abi::object::ProgramRequirement;
use sapio::contract::Compiled;
use sapio_contrib::contracts::eltoo::State;
use sapio_contrib::contracts::template_authorization::Authorization;
use sapio_integration_tests::eltoo_example::{fixture, runner};
use sapio_integration_tests::fragment_example::{
    authorization_requirement, compile_candidates, example_contract, participant_key,
    signing_request,
};
use sapio_integration_tests::program_example::{bind_candidates, example_root};
use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio};

#[derive(Serialize, Deserialize)]
struct SigningCase {
    artifact: Compiled,
    requirement: ProgramRequirement,
    psbt: PSBT,
    witness: Vec<u8>,
    sponsor: bool,
    witness_items: Vec<usize>,
}

impl SigningCase {
    fn new(
        mut artifact: Compiled,
        requirement: ProgramRequirement,
        mut request: ProgramSigningRequest,
        sponsor: bool,
        witness_items: Vec<usize>,
    ) -> Self {
        assert_eq!(request.instance, *requirement.program.instance());
        assert_eq!(request.path, requirement.path);
        // Signing must reconstruct descriptor proofs from the artifact and
        // cannot depend on the example's optional descriptive metadata.
        artifact.metadata = Default::default();
        let input = &mut request.psbt.0.inputs[0];
        input.tap_internal_key = None;
        input.tap_merkle_root = None;
        input.tap_scripts.clear();
        input.tap_key_origins.clear();
        Self {
            artifact,
            requirement,
            psbt: request.psbt,
            witness: request.witness,
            sponsor,
            witness_items,
        }
    }
}

fn exported_cases() -> Vec<SigningCase> {
    let secp = Secp256k1::new();
    let root = example_root();
    let participant = participant_key();
    let mut cases = vec![];
    for (mode, signer, witness_items) in [
        (
            Authorization::InternalKey(participant.x_only_public_key().0),
            participant,
            4,
        ),
        (Authorization::KnownTweak, root.to_keypair(&secp), 2),
    ] {
        let compiled = {
            let contract = example_contract(mode, Xpub::from_priv(&secp, &root));
            compile_candidates(&contract, &[]).unwrap()
        };
        let candidate = bind_candidates(&compiled).unwrap().remove(0);
        let requirement = authorization_requirement(&compiled, mode).unwrap();
        let request = signing_request(
            &compiled,
            mode,
            candidate,
            &signer,
            Some(b"\x50exported-fragment".to_vec()),
        )
        .unwrap();
        cases.push(SigningCase::new(
            compiled,
            requirement,
            request,
            false,
            vec![witness_items],
        ));
    }

    let (update, settlement, update_coin, settlement_coin, authorization) = {
        let terms = fixture::terms();
        let source = terms
            .state(State {
                number: 1,
                alice_sats: 40_000,
            })
            .unwrap();
        let target = State {
            number: 2,
            alice_sats: 60_000,
        };
        (
            runner::compile_update(&source, target).unwrap(),
            runner::compile_settlement(&source).unwrap(),
            fixture::coin(&source, 70),
            fixture::coin(&source, 71),
            runner::authorize_update(&terms, target, &fixture::joint_key()).unwrap(),
        )
    };
    // Both the source Channel and Terms have gone out of scope. Binding and
    // requirement selection now use only the exported artifacts and evidence.
    let requirement = runner::update_requirement(&update).unwrap();
    let request = runner::update_request_from_artifact(
        &update,
        update_coin,
        fixture::sponsor(2_000, 72),
        &authorization,
    )
    .unwrap();
    cases.push(SigningCase::new(
        update,
        requirement,
        request,
        true,
        vec![3, 1],
    ));
    let request = runner::settlement_request_from_artifact(
        &settlement,
        settlement_coin,
        fixture::sponsor(3_000, 73),
    )
    .unwrap();
    let requirement =
        runner::settlement_requirement(&settlement, &request.psbt.0.unsigned_tx).unwrap();
    cases.push(SigningCase::new(
        settlement,
        requirement,
        request,
        true,
        vec![3, 1],
    ));
    cases
}

fn sign_exports() {
    let cases: Vec<SigningCase> = serde_json::from_reader(std::io::stdin().lock()).unwrap();
    assert_eq!(cases.len(), 4);
    // Oracle configuration is local to the signing process, independent of
    // the imported artifacts. These keys are disposable test fixtures.
    let oracles = [
        ProgramOracle::new(example_root(), vec![]).unwrap(),
        ProgramOracle::new(fixture::oracle_key(), vec![]).unwrap(),
    ];
    for case in cases {
        assert_eq!(case.artifact.metadata, Default::default());
        let unsigned = case.psbt.0.unsigned_tx.clone();
        let annex = sapio_psbt::annex::get(&case.psbt.0.inputs[0])
            .unwrap()
            .map(Vec::from);
        let oracle = oracles
            .iter()
            .find(|oracle| oracle.public_root() == *case.requirement.program.root())
            .unwrap();
        let request = prepare_program_request(
            &case.artifact,
            &case.requirement,
            case.psbt.0,
            0,
            case.witness,
        )
        .unwrap();
        let mut signed = oracle.sign(request.clone()).unwrap();
        validate_program_response(&request, &signed, case.requirement.program.root()).unwrap();
        if case.sponsor {
            runner::sign_sponsor(&mut signed, &fixture::sponsor_key()).unwrap();
        }
        let finalized = sapio_psbt::finalize::finalize(signed, &Secp256k1::new()).unwrap();
        let transaction = finalized.extract_tx().unwrap();
        assert_eq!(transaction.compute_txid(), unsigned.compute_txid());
        assert_eq!(
            transaction
                .input
                .iter()
                .map(|input| input.witness.len())
                .collect::<Vec<_>>(),
            case.witness_items
        );
        if let Some(annex) = annex {
            assert_eq!(transaction.input[0].witness.last(), Some(annex.as_slice()));
        }
    }
}

#[test]
fn exported_programs_sign_in_a_fresh_process() {
    if std::env::var_os("SAPIO_ARTIFACT_SIGNING_CHILD").is_some() {
        sign_exports();
        return;
    }
    let cases = exported_cases();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "exported_programs_sign_in_a_fresh_process",
            "--nocapture",
        ])
        .env("SAPIO_ARTIFACT_SIGNING_CHILD", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let sent = serde_json::to_writer(child.stdin.take().unwrap(), &cases);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "child failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    sent.unwrap();
}
