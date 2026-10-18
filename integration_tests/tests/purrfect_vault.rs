use bitcoin::consensus::serialize;
use bitcoin::psbt::{Input, Output, Psbt};
use bitcoin::{Amount, OutPoint, ScriptBuf, Sequence, Transaction};
use emulator_connect::program::{
    prepare_program_request, ProgramError, ProgramOracle, ProgramSigningRequest, SpendIntent,
};
use miniscript::Descriptor;
use sapio::contract::object::SupportedDescriptors;
use sapio::contract::Compiled;
use sapio_base::secp::secp;
use sapio_contrib::contracts::purrfect_vault::{complete_evidence, Stage, MARKER_SATS};
use sapio_integration_tests::purrfect_vault_example::*;
use sapio_psbt::SigningKey;

fn oracle() -> ProgramOracle {
    ProgramOracle::new(oracle_key(), vec![]).unwrap()
}

// Use policy-only artifacts for adversarial transactions. Their preparation
// verifies the actual spending proof without a candidate's local fee policy
// rejecting a mutation before the real WASM predicate can inspect it.
fn request(
    policies: &Compiled,
    stage: Stage,
    psbt: Psbt,
    input_index: u32,
    witness: Vec<u8>,
) -> ProgramSigningRequest {
    prepare_program_request(
        policies,
        &requirement(policies, &vault().unwrap(), stage).unwrap(),
        psbt,
        input_index,
        witness,
    )
    .expect("the adversarial transaction must reach the WASM evaluator")
}

fn rejected(policies: &Compiled, stage: Stage, psbt: Psbt, witness: Vec<u8>) {
    assert!(matches!(
        oracle().sign(request(policies, stage, psbt, 0, witness)),
        Err(ProgramError::Rejected)
    ));
}

fn malformed(policies: &Compiled, stage: Stage, psbt: Psbt, witness: Vec<u8>) {
    assert!(matches!(
        oracle().sign(request(policies, stage, psbt, 0, witness)),
        Err(ProgramError::Evaluation(_))
    ));
}

fn portable_finish(
    compiled: &Compiled,
    intent: &SpendIntent,
    oracle: &ProgramOracle,
) -> Transaction {
    let restored: Compiled =
        serde_json::from_slice(&serde_json::to_vec(compiled).unwrap()).unwrap();
    let saved: SpendIntent = serde_json::from_slice(&serde_json::to_vec(intent).unwrap()).unwrap();
    restored.validate().unwrap();
    saved.status(&restored, saved.baseline_psbt()).unwrap();
    finish(&restored, &saved, oracle).unwrap()
}

fn trigger() -> Transaction {
    let source = vault().unwrap();
    let compiled = compile_trigger(&source, destination(124).script_pubkey()).unwrap();
    execute(
        &compiled,
        &source,
        Stage::Trigger,
        funding_coin(&compiled),
        sponsor_coin(10),
        None,
        &oracle(),
    )
    .unwrap()
}

#[test]
fn portable_actions_link_trigger_cancel_retrigger_and_complete_at_one_address() {
    let source = vault().unwrap();
    let bare = compile(&source).unwrap();
    let vault_script: ScriptBuf = (&bare.address).into();
    assert!(bare.suggested_txs.is_empty());
    assert_eq!(bare.program_requirements().unwrap().len(), 3);
    let Some(SupportedDescriptors::XOnly(Descriptor::Tr(tree))) = bare.descriptor.as_ref() else {
        panic!("vault must compile to a Miniscript Taproot descriptor");
    };
    assert_eq!(tree.leaves().count(), 3);
    assert_ne!(*tree.internal_key(), source.owner());
    for stage in [Stage::Trigger, Stage::Cancel, Stage::Complete] {
        assert_ne!(
            *tree.internal_key(),
            source.program(stage).unwrap().derive_public_key().unwrap()
        );
    }

    let oracle = oracle();
    let first_destination = destination(124).script_pubkey();
    let second_destination = destination(125).script_pubkey();
    let first = compile_trigger(&source, first_destination.clone()).unwrap();
    let bound = bind(&first, funding_coin(&first), sponsor_coin(11)).unwrap();
    let first_id = bound.unsigned_tx.compute_txid();
    let intent = prepare(&first, &source, Stage::Trigger, bound, vec![]).unwrap();
    let triggered = portable_finish(&first, &intent, &oracle);
    assert_eq!(triggered.compute_txid(), first_id);
    assert_eq!(triggered.output[1].value.to_sat(), MARKER_SATS);
    assert_eq!(triggered.output[1].script_pubkey, first_destination);

    let cancel = compile_cancel(&source).unwrap();
    let bound = bind(&cancel, principal_coin(&triggered), sponsor_coin(12)).unwrap();
    let intent = prepare(&cancel, &source, Stage::Cancel, bound, vec![]).unwrap();
    let cancelled = portable_finish(&cancel, &intent, &oracle);
    assert_eq!(
        cancelled.input[0].previous_output,
        OutPoint::new(triggered.compute_txid(), 0)
    );
    assert_eq!(cancelled.output.len(), 1);
    assert!(compile_complete(&source, &cancelled).is_err());

    let retrigger = compile_trigger(&source, second_destination.clone()).unwrap();
    let bound = bind(&retrigger, principal_coin(&cancelled), sponsor_coin(13)).unwrap();
    let intent = prepare(&retrigger, &source, Stage::Trigger, bound, vec![]).unwrap();
    let retriggered = portable_finish(&retrigger, &intent, &oracle);
    assert_eq!(
        retriggered.input[0].previous_output,
        OutPoint::new(cancelled.compute_txid(), 0)
    );
    assert_eq!(retriggered.output[1].script_pubkey, second_destination);

    let complete = compile_complete(&source, &retriggered).unwrap();
    let bound = bind(&complete, principal_coin(&retriggered), sponsor_coin(14)).unwrap();
    let intent = prepare(
        &complete,
        &source,
        Stage::Complete,
        bound,
        complete_evidence(&retriggered).unwrap(),
    )
    .unwrap();
    let completed = portable_finish(&complete, &intent, &oracle);
    assert_eq!(
        completed.input[0].previous_output,
        OutPoint::new(retriggered.compute_txid(), 0)
    );
    assert_eq!(completed.input[0].sequence, Sequence::from_height(20));
    assert_eq!(completed.output.len(), 1);
    assert_eq!(completed.output[0].script_pubkey, second_destination);

    for compiled in [&first, &cancel, &retrigger, &complete] {
        assert_eq!(compiled.address, bare.address);
        assert_eq!(compiled.descriptor, bare.descriptor);
        assert_eq!(compiled.program_policies, bare.program_policies);
        assert!(compiled.ctv_to_tx.is_empty());
        assert!(!compiled.requires_native_ctv());
        assert_eq!(compiled.required_input_amount.to_sat(), PRINCIPAL_SATS);
    }
    for transaction in [&triggered, &cancelled, &retriggered, &completed] {
        assert_eq!(transaction.output[0].value.to_sat(), PRINCIPAL_SATS);
        assert!(transaction
            .input
            .iter()
            .all(|input| !input.witness.is_empty()));
    }
    for transaction in [&triggered, &cancelled, &retriggered] {
        assert_eq!(transaction.output[0].script_pubkey, vault_script);
    }
}

#[test]
fn every_branch_requires_both_native_owner_and_oracle_signatures() {
    let source = vault().unwrap();
    let parent = trigger();
    let oracle = oracle();
    for stage in [Stage::Trigger, Stage::Cancel, Stage::Complete] {
        let compiled = match stage {
            Stage::Trigger => compile_trigger(&source, destination(126).script_pubkey()),
            Stage::Cancel => compile_cancel(&source),
            Stage::Complete => compile_complete(&source, &parent),
        }
        .unwrap();
        let witness = if stage == Stage::Complete {
            complete_evidence(&parent).unwrap()
        } else {
            vec![]
        };
        let bound = bind(&compiled, principal_coin(&parent), sponsor_coin(15)).unwrap();
        let intent = prepare(&compiled, &source, stage, bound, witness).unwrap();
        assert_eq!(intent.requests().len(), 1);

        let mut owner_only = intent.baseline_psbt().clone();
        intent
            .sign_native(
                &compiled,
                &mut owner_only,
                &SigningKey(vec![owner_key()]),
                secp(),
            )
            .unwrap();
        sign_sponsor(&mut owner_only).unwrap();
        assert!(intent.finalize(&compiled, &owner_only, secp()).is_err());
        assert!(sapio_psbt::finalize::finalize(owner_only, secp()).is_err());

        let mut oracle_only = intent.baseline_psbt().clone();
        let response = oracle.sign(intent.requests()[0].clone()).unwrap();
        intent
            .merge_response(&compiled, &mut oracle_only, 0, &response)
            .unwrap();
        sign_sponsor(&mut oracle_only).unwrap();
        assert!(intent.finalize(&compiled, &oracle_only, secp()).is_err());
        assert!(sapio_psbt::finalize::finalize(oracle_only, secp()).is_err());

        let signed = sign(&compiled, &intent, &oracle).unwrap();
        assert!(intent.finalize(&compiled, &signed, secp()).is_ok());
        let mut changed = signed;
        changed.unsigned_tx.output[0].value -= Amount::ONE_SAT;
        // Reauthorize the sponsor so rejection also proves that the vault's
        // collected signatures cannot authorize changed transaction outputs.
        sign_sponsor(&mut changed).unwrap();
        assert!(intent.finalize(&compiled, &changed, secp()).is_err());
        assert!(sapio_psbt::finalize::finalize(changed, secp()).is_err());
    }
}

#[test]
fn the_guest_preserves_principal_marker_and_the_single_vault_input_profile() {
    let source = vault().unwrap();
    let policies = compile(&source).unwrap();
    let compiled = compile_trigger(&source, destination(124).script_pubkey()).unwrap();
    let bound = bind(&compiled, funding_coin(&compiled), sponsor_coin(16)).unwrap();
    oracle()
        .sign(request(&policies, Stage::Trigger, bound.clone(), 0, vec![]))
        .unwrap();

    let mut principal = bound.clone();
    principal.unsigned_tx.output[0].value -= Amount::ONE_SAT;
    rejected(&policies, Stage::Trigger, principal, vec![]);
    let mut destination_changed = bound.clone();
    destination_changed.unsigned_tx.output[0].script_pubkey = destination(125).script_pubkey();
    rejected(&policies, Stage::Trigger, destination_changed, vec![]);
    let mut marker = bound.clone();
    marker.unsigned_tx.output[1].value -= Amount::ONE_SAT;
    rejected(&policies, Stage::Trigger, marker, vec![]);
    let mut extra_output = bound.clone();
    extra_output.unsigned_tx.output.push(bitcoin::TxOut {
        value: Amount::ZERO,
        script_pubkey: destination(125).script_pubkey(),
    });
    extra_output.outputs.push(Output::default());
    rejected(&policies, Stage::Trigger, extra_output, vec![]);
    malformed(&policies, Stage::Trigger, bound.clone(), vec![0]);

    let mut extra_input = bound.clone();
    let sponsor = sponsor_coin(17);
    let mut txin = extra_input.unsigned_tx.input[1].clone();
    txin.previous_output = sponsor.outpoint;
    extra_input.unsigned_tx.input.push(txin);
    extra_input.inputs.push(Input {
        witness_utxo: Some(sponsor.txout),
        ..Default::default()
    });
    rejected(&policies, Stage::Trigger, extra_input, vec![]);
    let mut reordered = bound.clone();
    reordered.unsigned_tx.input.swap(0, 1);
    reordered.inputs.swap(0, 1);
    assert!(matches!(
        oracle().sign(request(&policies, Stage::Trigger, reordered, 1, vec![])),
        Err(ProgramError::Rejected)
    ));
    let mut second_vault = bound.clone();
    second_vault.inputs[1]
        .witness_utxo
        .as_mut()
        .unwrap()
        .script_pubkey = (&policies.address).into();
    malformed(&policies, Stage::Trigger, second_vault, vec![]);
    let mut annex = bound;
    sapio_psbt::annex::set(&mut annex.inputs[0], Some(vec![0x50, 1])).unwrap();
    rejected(&policies, Stage::Trigger, annex, vec![]);

    let cancel = compile_cancel(&source).unwrap();
    let bound = bind(&cancel, funding_coin(&cancel), sponsor_coin(18)).unwrap();
    let mut changed = bound.clone();
    changed.unsigned_tx.output[0].script_pubkey = destination(125).script_pubkey();
    rejected(&policies, Stage::Cancel, changed, vec![]);
    let mut changed = bound;
    changed.unsigned_tx.output[0].value -= Amount::ONE_SAT;
    rejected(&policies, Stage::Cancel, changed, vec![]);
}

#[test]
fn completion_authenticates_the_immediate_parent_and_canonical_stripped_evidence() {
    let source = vault().unwrap();
    let policies = compile(&source).unwrap();
    let parent = trigger();
    let compiled = compile_complete(&source, &parent).unwrap();
    let bound = bind(&compiled, principal_coin(&parent), sponsor_coin(19)).unwrap();
    let proof = complete_evidence(&parent).unwrap();
    oracle()
        .sign(request(
            &policies,
            Stage::Complete,
            bound.clone(),
            0,
            proof.clone(),
        ))
        .unwrap();

    let mut wrong_parent = bound.clone();
    wrong_parent.unsigned_tx.input[0].previous_output.txid = funding_coin(&compiled).outpoint.txid;
    rejected(&policies, Stage::Complete, wrong_parent, proof.clone());
    let mut wrong_output = bound.clone();
    wrong_output.unsigned_tx.input[0].previous_output.vout = 1;
    rejected(&policies, Stage::Complete, wrong_output, proof.clone());
    let mut wrong_destination = bound.clone();
    wrong_destination.unsigned_tx.output[0].script_pubkey = destination(125).script_pubkey();
    rejected(&policies, Stage::Complete, wrong_destination, proof.clone());
    let mut changed_proof = proof.clone();
    changed_proof[0] ^= 1;
    rejected(&policies, Stage::Complete, bound.clone(), changed_proof);
    let mut trailing = proof.clone();
    trailing.push(0);
    malformed(&policies, Stage::Complete, bound.clone(), trailing);
    malformed(
        &policies,
        Stage::Complete,
        bound.clone(),
        proof[..proof.len() - 1].to_vec(),
    );
    let mut witnessed = parent.clone();
    for input in &mut witnessed.input {
        input.witness.clear();
    }
    witnessed.input[0].witness.push([1]);
    assert_eq!(complete_evidence(&witnessed).unwrap(), proof);
    malformed(&policies, Stage::Complete, bound, serialize(&witnessed));

    let cancel = compile_cancel(&source).unwrap();
    let cancelled = execute(
        &cancel,
        &source,
        Stage::Cancel,
        principal_coin(&parent),
        sponsor_coin(20),
        None,
        &oracle(),
    )
    .unwrap();
    assert!(compile_complete(&source, &cancelled).is_err());
    let after_cancel = bind(&compiled, principal_coin(&cancelled), sponsor_coin(21)).unwrap();
    rejected(&policies, Stage::Complete, after_cancel, proof);
}

#[test]
fn completion_requires_a_twenty_block_relative_lock_in_the_signed_transaction() {
    let source = vault().unwrap();
    let policies = compile(&source).unwrap();
    let parent = trigger();
    let compiled = compile_complete(&source, &parent).unwrap();
    let bound = bind(&compiled, principal_coin(&parent), sponsor_coin(22)).unwrap();
    let proof = complete_evidence(&parent).unwrap();
    for sequence in [19, (1 << 31) | 20, (1 << 22) | 20] {
        let mut changed = bound.clone();
        changed.unsigned_tx.input[0].sequence = Sequence::from_consensus(sequence);
        rejected(&policies, Stage::Complete, changed, proof.clone());
    }
    let mut version_one = bound.clone();
    version_one.unsigned_tx.version = bitcoin::transaction::Version::ONE;
    rejected(&policies, Stage::Complete, version_one, proof.clone());
    let mut longer_delay = bound;
    longer_delay.unsigned_tx.input[0].sequence = Sequence::from_height(21);
    oracle()
        .sign(request(&policies, Stage::Complete, longer_delay, 0, proof))
        .unwrap();
}
