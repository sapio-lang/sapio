use bitcoin::hashes::Hash;
use bitcoin::psbt::{Input, Output, Psbt};
use bitcoin::secp256k1::Message;
use bitcoin::sighash::{Prevouts, SighashCache};
use bitcoin::taproot::Signature;
use bitcoin::{Amount, OutPoint, ScriptBuf, Sequence, TapSighashType, Transaction};
use emulator_connect::program::{
    prepare_program_request, ProgramError, ProgramOracle, ProgramSigningRequest, SpendIntent,
};
use miniscript::Descriptor;
use sapio::contract::object::SupportedDescriptors;
use sapio::contract::Compiled;
use sapio_base::bitcoin_script::encode_stack;
use sapio_base::program::ProgramSpendPath;
use sapio_base::secp::secp;
use sapio_contrib::contracts::purrfect_vault::{
    script_witness, stripped_trigger, synthetic_signature, Stage, MARKER_SATS,
};
use sapio_integration_tests::purrfect_vault_example::*;
use sapio_psbt::SigningKey;

fn oracle() -> ProgramOracle {
    ProgramOracle::new(oracle_key(), vec![]).unwrap()
}

// Policy-only artifacts let adversarial transactions reach the actual Script
// interpreter without a suggested candidate's local funding policy rejecting
// them first. Recreate both signatures for the changed transaction so failure
// proves its CAT covenant, rather than merely detecting a stale owner signature.
fn request(
    policies: &Compiled,
    stage: Stage,
    mut psbt: Psbt,
    parent: Option<&Transaction>,
) -> ProgramSigningRequest {
    let requirement = requirement(policies, &vault().unwrap(), stage).unwrap();
    let ProgramSpendPath::ScriptPath(leaf) = requirement.path else {
        panic!("vault branches require a script path");
    };
    for nonce in 0..4_096 {
        psbt.unsigned_tx.input[1].sequence = Sequence::from_consensus((1 << 31) | nonce);
        if synthetic_signature(&psbt, leaf).unwrap().is_some() {
            let prevouts: Vec<_> = psbt
                .inputs
                .iter()
                .map(|input| input.witness_utxo.clone().unwrap())
                .collect();
            let digest = SighashCache::new(&psbt.unsigned_tx)
                .taproot_script_spend_signature_hash(
                    0,
                    &Prevouts::All(&prevouts),
                    leaf,
                    TapSighashType::All,
                )
                .unwrap();
            let owner = Signature {
                signature: secp().sign_schnorr_no_aux_rand(
                    &Message::from_digest(digest.to_byte_array()),
                    &owner_key().to_keypair(secp()),
                ),
                sighash_type: TapSighashType::All,
            };
            let stack = script_witness(stage, &psbt, leaf, parent, owner).unwrap();
            return prepare_program_request(
                policies,
                &requirement,
                psbt,
                0,
                encode_stack(&stack).unwrap(),
            )
            .expect("adversarial transaction must reach the Script evaluator");
        }
    }
    panic!("could not grind the adversarial transaction");
}

fn denied(request: ProgramSigningRequest) {
    assert!(matches!(
        oracle().sign(request),
        Err(ProgramError::Rejected | ProgramError::Evaluation(_))
    ));
}

fn stack_items(bytes: &[u8]) -> Vec<Vec<u8>> {
    let count = u32::from_le_bytes(bytes[..4].try_into().unwrap());
    let mut position = 4;
    (0..count)
        .map(|_| {
            let length =
                u32::from_le_bytes(bytes[position..position + 4].try_into().unwrap()) as usize;
            position += 4;
            let value = bytes[position..position + length].to_vec();
            position += length;
            value
        })
        .collect()
}

#[test]
fn script_constructs_canonical_destination_lengths() {
    let source = vault().unwrap();
    let policies = compile(&source).unwrap();
    let mut compiled = compile_trigger(&source, ScriptBuf::from_bytes(vec![0x51; 127])).unwrap();
    let coin = funding_coin(&compiled);
    let bound = bind(
        &mut compiled,
        &source,
        Stage::Trigger,
        coin,
        sponsor_coin(41),
    )
    .unwrap();
    oracle()
        .sign(request(&policies, Stage::Trigger, bound.clone(), None))
        .unwrap();
    for length in [0, 128] {
        let mut invalid = bound.clone();
        invalid.unsigned_tx.output[1].script_pubkey = ScriptBuf::from_bytes(vec![0x51; length]);
        denied(request(&policies, Stage::Trigger, invalid, None));
    }
}

#[test]
fn script_enforces_signature_split_and_amount_field_boundaries() {
    let source = vault().unwrap();
    let policies = compile(&source).unwrap();
    let mut cancellation = compile_cancel(&source).unwrap();
    let coin = funding_coin(&cancellation);
    let bound = bind(
        &mut cancellation,
        &source,
        Stage::Cancel,
        coin,
        sponsor_coin(40),
    )
    .unwrap();
    let mut split = request(&policies, Stage::Cancel, bound.clone(), None);
    oracle().sign(split.clone()).unwrap();
    let mut stack = stack_items(&split.witness);
    let prefix = stack.len() - 4;
    let mut reconstructed = stack[prefix].clone();
    reconstructed.extend_from_slice(&stack[prefix + 1]);
    let previous: Vec<_> = split
        .psbt
        .0
        .inputs
        .iter()
        .map(|input| input.witness_utxo.clone().unwrap())
        .collect();
    let ProgramSpendPath::ScriptPath(leaf) = split.path else {
        unreachable!()
    };
    let digest = SighashCache::new(&split.psbt.0.unsigned_tx)
        .taproot_script_spend_signature_hash(
            0,
            &Prevouts::All(&previous),
            leaf,
            TapSighashType::Default,
        )
        .unwrap();
    let mut one = [0; 32];
    one[31] = 1;
    let generator = bitcoin::secp256k1::Keypair::from_secret_key(
        secp(),
        &bitcoin::secp256k1::SecretKey::from_slice(&one).unwrap(),
    );
    let replacement =
        secp().sign_schnorr_no_aux_rand(&Message::from_digest(digest.to_byte_array()), &generator);
    secp()
        .verify_schnorr(
            &replacement,
            &Message::from_digest(digest.to_byte_array()),
            &generator.x_only_public_key().0,
        )
        .unwrap();
    stack[prefix].clear();
    stack[prefix + 1] = reconstructed;
    stack[prefix + 2] = replacement.as_ref().to_vec();
    split.witness = encode_stack(&stack).unwrap();
    assert!(matches!(oracle().sign(split), Err(ProgramError::Rejected)));

    // Preserve the input-amount hash while shifting a byte into the output's
    // script-length field. Explicit eight-byte amounts must reject this split.
    let mut shifted = bound;
    shifted.inputs[1].witness_utxo.as_mut().unwrap().value = Amount::from_sat(2_083);
    let vault_script = shifted.inputs[0]
        .witness_utxo
        .as_ref()
        .unwrap()
        .script_pubkey
        .as_bytes();
    shifted.unsigned_tx.output[0].script_pubkey =
        ScriptBuf::from_bytes([&[0x22][..], vault_script].concat());
    let mut shifted = request(&policies, Stage::Cancel, shifted, None);
    let mut stack = stack_items(&shifted.witness);
    let amounts = [stack[10].as_slice(), stack[12].as_slice()].concat();
    assert_eq!(stack[12][0], 0x23);
    let shifted_byte = stack[12].remove(0);
    stack[10].push(shifted_byte);
    assert_eq!(
        [stack[10].as_slice(), stack[12].as_slice()].concat(),
        amounts
    );
    assert_eq!(
        [stack[10].as_slice(), stack[11].as_slice()].concat(),
        bitcoin::consensus::serialize(&shifted.psbt.0.unsigned_tx.output[0])
    );
    shifted.witness = encode_stack(&stack).unwrap();
    assert!(matches!(
        oracle().sign(shifted),
        Err(ProgramError::Rejected)
    ));
}

fn bind_candidate(compiled: &mut Compiled, stage: Stage, principal: Coin, sponsor: Coin) -> Psbt {
    let psbt = bind(compiled, &vault().unwrap(), stage, principal, sponsor).unwrap();
    candidate(compiled)
        .unwrap()
        .check_funded_psbt(&psbt)
        .unwrap();
    assert_eq!(
        candidate(compiled).unwrap().tx.input[1].sequence,
        psbt.unsigned_tx.input[1].sequence
    );
    psbt
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
    let mut first = compile_trigger(&source, first_destination.clone()).unwrap();
    let principal = funding_coin(&first);
    let bound = bind_candidate(&mut first, Stage::Trigger, principal, sponsor_coin(11));
    let first_id = bound.unsigned_tx.compute_txid();
    let intent = prepare(&first, &source, Stage::Trigger, bound, None).unwrap();
    let triggered = portable_finish(&first, &intent, &oracle);
    assert_eq!(triggered.compute_txid(), first_id);
    assert_eq!(triggered.output[1].value.to_sat(), MARKER_SATS);
    assert_eq!(triggered.output[1].script_pubkey, first_destination);

    let mut cancel = compile_cancel(&source).unwrap();
    let bound = bind_candidate(
        &mut cancel,
        Stage::Cancel,
        principal_coin(&triggered),
        sponsor_coin(12),
    );
    let intent = prepare(&cancel, &source, Stage::Cancel, bound, None).unwrap();
    let cancelled = portable_finish(&cancel, &intent, &oracle);
    assert_eq!(
        cancelled.input[0].previous_output,
        OutPoint::new(triggered.compute_txid(), 0)
    );
    assert_eq!(cancelled.output.len(), 1);
    assert!(compile_complete(&source, &cancelled).is_err());

    let mut retrigger = compile_trigger(&source, second_destination.clone()).unwrap();
    let bound = bind_candidate(
        &mut retrigger,
        Stage::Trigger,
        principal_coin(&cancelled),
        sponsor_coin(13),
    );
    let intent = prepare(&retrigger, &source, Stage::Trigger, bound, None).unwrap();
    let retriggered = portable_finish(&retrigger, &intent, &oracle);
    assert_eq!(
        retriggered.input[0].previous_output,
        OutPoint::new(cancelled.compute_txid(), 0)
    );
    assert_eq!(retriggered.output[1].script_pubkey, second_destination);

    let mut complete = compile_complete(&source, &retriggered).unwrap();
    let bound = bind_candidate(
        &mut complete,
        Stage::Complete,
        principal_coin(&retriggered),
        sponsor_coin(14),
    );
    let intent = prepare(
        &complete,
        &source,
        Stage::Complete,
        bound,
        Some(&retriggered),
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
        let mut compiled = match stage {
            Stage::Trigger => compile_trigger(&source, destination(126).script_pubkey()),
            Stage::Cancel => compile_cancel(&source),
            Stage::Complete => compile_complete(&source, &parent),
        }
        .unwrap();
        let witness = (stage == Stage::Complete).then_some(&parent);
        let bound = bind_candidate(
            &mut compiled,
            stage,
            principal_coin(&parent),
            sponsor_coin(15),
        );
        let intent = prepare(&compiled, &source, stage, bound, witness).unwrap();
        assert_eq!(intent.requests().len(), 1);
        assert!(intent.baseline_psbt().inputs[0].tap_script_sigs.is_empty());
        assert!(intent.baseline_psbt().inputs[0]
            .tap_scripts
            .values()
            .all(
                |(script, _)| script.instructions().all(|instruction| !matches!(
                    instruction,
                    Ok(bitcoin::script::Instruction::Op(
                        bitcoin::opcodes::all::OP_CAT
                    ))
                ))
            ));

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
fn executed_cat_scripts_preserve_principal_marker_and_the_vault_profile() {
    let source = vault().unwrap();
    let policies = compile(&source).unwrap();
    let mut compiled = compile_trigger(&source, destination(124).script_pubkey()).unwrap();
    let principal = funding_coin(&compiled);
    let bound = bind_candidate(&mut compiled, Stage::Trigger, principal, sponsor_coin(16));
    let valid = request(&policies, Stage::Trigger, bound.clone(), None);
    oracle().sign(valid.clone()).unwrap();

    let mut principal = bound.clone();
    principal.unsigned_tx.output[0].value -= Amount::ONE_SAT;
    denied(request(&policies, Stage::Trigger, principal, None));
    let mut changed_address = bound.clone();
    changed_address.unsigned_tx.output[0].script_pubkey = destination(125).script_pubkey();
    denied(request(&policies, Stage::Trigger, changed_address, None));
    let mut marker = bound.clone();
    marker.unsigned_tx.output[1].value -= Amount::ONE_SAT;
    denied(request(&policies, Stage::Trigger, marker, None));
    let mut extra_output = bound;
    extra_output.unsigned_tx.output.push(bitcoin::TxOut {
        value: Amount::ZERO,
        script_pubkey: destination(125).script_pubkey(),
    });
    extra_output.outputs.push(Output::default());
    denied(request(&policies, Stage::Trigger, extra_output, None));

    // Profile violations terminate before Script signatures are evaluated.
    let mut extra_input = valid.psbt.0.clone();
    let sponsor = sponsor_coin(17);
    let mut txin = extra_input.unsigned_tx.input[1].clone();
    txin.previous_output = sponsor.outpoint;
    extra_input.unsigned_tx.input.push(txin);
    extra_input.inputs.push(Input {
        witness_utxo: Some(sponsor.txout),
        ..Default::default()
    });
    denied(
        prepare_program_request(
            &policies,
            &requirement(&policies, &source, Stage::Trigger).unwrap(),
            extra_input,
            0,
            valid.witness.clone(),
        )
        .unwrap(),
    );
    let mut second_vault = valid.clone();
    second_vault.psbt.0.inputs[1]
        .witness_utxo
        .as_mut()
        .unwrap()
        .script_pubkey = (&policies.address).into();
    denied(second_vault);
    let mut annex = valid;
    sapio_psbt::annex::set(&mut annex.psbt.0.inputs[0], Some(vec![0x50, 1])).unwrap();
    denied(annex);

    let mut cancel = compile_cancel(&source).unwrap();
    let principal = funding_coin(&cancel);
    let bound = bind_candidate(&mut cancel, Stage::Cancel, principal, sponsor_coin(18));
    oracle()
        .sign(request(&policies, Stage::Cancel, bound.clone(), None))
        .unwrap();
    let mut changed = bound.clone();
    changed.unsigned_tx.output[0].script_pubkey = destination(125).script_pubkey();
    denied(request(&policies, Stage::Cancel, changed, None));
    let mut changed = bound;
    changed.unsigned_tx.output[0].value -= Amount::ONE_SAT;
    denied(request(&policies, Stage::Cancel, changed, None));
}

#[test]
fn executed_completion_authenticates_parent_output_and_destination() {
    let source = vault().unwrap();
    let policies = compile(&source).unwrap();
    let parent = trigger();
    let mut compiled = compile_complete(&source, &parent).unwrap();
    let bound = bind_candidate(
        &mut compiled,
        Stage::Complete,
        principal_coin(&parent),
        sponsor_coin(19),
    );
    oracle()
        .sign(request(
            &policies,
            Stage::Complete,
            bound.clone(),
            Some(&parent),
        ))
        .unwrap();

    let mut wrong_parent = bound.clone();
    wrong_parent.unsigned_tx.input[0].previous_output.txid = funding_coin(&compiled).outpoint.txid;
    denied(request(
        &policies,
        Stage::Complete,
        wrong_parent,
        Some(&parent),
    ));
    let mut wrong_output = bound.clone();
    wrong_output.unsigned_tx.input[0].previous_output.vout = 1;
    denied(request(
        &policies,
        Stage::Complete,
        wrong_output,
        Some(&parent),
    ));
    let mut wrong_amount = bound.clone();
    wrong_amount.unsigned_tx.output[0].value -= Amount::ONE_SAT;
    denied(request(
        &policies,
        Stage::Complete,
        wrong_amount,
        Some(&parent),
    ));
    let mut wrong_destination = bound.clone();
    wrong_destination.unsigned_tx.output[0].script_pubkey = destination(125).script_pubkey();
    denied(request(
        &policies,
        Stage::Complete,
        wrong_destination,
        Some(&parent),
    ));
    let mut changed_parent = parent.clone();
    changed_parent.version = bitcoin::transaction::Version::ONE;
    denied(request(
        &policies,
        Stage::Complete,
        bound,
        Some(&changed_parent),
    ));

    let mut witnessed = parent.clone();
    witnessed.input[0].witness.push([1]);
    assert_eq!(
        stripped_trigger(&witnessed).unwrap(),
        stripped_trigger(&parent).unwrap()
    );
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
    let after_cancel = bind_candidate(
        &mut compiled,
        Stage::Complete,
        principal_coin(&cancelled),
        sponsor_coin(21),
    );
    denied(request(
        &policies,
        Stage::Complete,
        after_cancel,
        Some(&parent),
    ));
}

#[test]
fn executed_csv_requires_twenty_blocks_with_the_correct_sequence_type() {
    let source = vault().unwrap();
    let policies = compile(&source).unwrap();
    let parent = trigger();
    let mut compiled = compile_complete(&source, &parent).unwrap();
    let bound = bind_candidate(
        &mut compiled,
        Stage::Complete,
        principal_coin(&parent),
        sponsor_coin(22),
    );
    for sequence in [19, (1 << 31) | 20, (1 << 22) | 20] {
        let mut changed = bound.clone();
        changed.unsigned_tx.input[0].sequence = Sequence::from_consensus(sequence);
        denied(request(&policies, Stage::Complete, changed, Some(&parent)));
    }
    let mut version_one = bound.clone();
    version_one.unsigned_tx.version = bitcoin::transaction::Version::ONE;
    denied(request(
        &policies,
        Stage::Complete,
        version_one,
        Some(&parent),
    ));
    let mut longer_delay = bound;
    longer_delay.unsigned_tx.input[0].sequence = Sequence::from_height(21);
    oracle()
        .sign(request(
            &policies,
            Stage::Complete,
            longer_delay,
            Some(&parent),
        ))
        .unwrap();
}
