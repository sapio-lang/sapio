//! End-to-end depot resolution through the program oracle: target reveal,
//! hit, claim binding, double-hit burns and the operator-free default branch.

use bitcoin::hashes::{sha256, Hash};
use bitcoin::Amount;
use sapio_contrib::contracts::depots::{ChannelBalance, ResolutionMode, Stage};
use sapio_integration_tests::depots_example::*;

type Result<T> = std::result::Result<T, Error>;

const TARGET: u64 = 3;

fn zeros() -> sha256::Hash {
    sha256::Hash::from_byte_array([0u8; 32])
}

/// Drive `F -> Targ -> Hit` with one hitting purchase, returning the Hit
/// transaction and the target proof used.
fn resolve_to_hit(
    terms: &sapio_contrib::contracts::depots::DepotTerms,
    oracle: &emulator_connect::program::ProgramOracle,
    tuple: &sapio_contrib::contracts::depots::GuessTuple,
    tag: u8,
) -> Result<(bitcoin::Transaction, (u64, [u8; 64]))> {
    let tasig = sign_target(terms, TARGET);

    let funding = compile_funding(terms, ResolutionMode::Operator)?;
    let targ_template = only_candidate(&funding)?.clone();
    let requirement = depot_requirement(
        &funding,
        terms,
        Stage::TargetReveal,
        ResolutionMode::Operator,
        commitment(&targ_template)?,
    )?;
    let coin = coin_for(&funding, terms.value(), tag);
    let input = input_from_artifact(&funding, &coin)?;
    let psbt = attach_inputs(
        targ_template.tx.clone(),
        coin,
        input,
        sponsor(2_000, tag + 1),
    )?;
    let intent = prepare_depot_spend(
        &funding,
        requirement,
        psbt,
        "depots/target/v1",
        sapio_contrib::contracts::depots::target_evidence(TARGET, &tasig),
    )?;
    let targ_tx = complete_candidate(&funding, &intent, oracle)?;
    assert_eq!(targ_tx.lock_time.to_consensus_u32(), EXPIRY);
    assert_eq!(targ_tx.output[0].value.to_sat(), DEPOT_SATS);

    let target_output = compile_target_output(terms, ResolutionMode::Operator)?;
    assert_eq!(
        targ_tx.output[0].script_pubkey,
        bitcoin::ScriptBuf::from(&target_output.address)
    );
    let hit_template = only_candidate(&target_output)?.clone();
    let requirement = depot_requirement(
        &target_output,
        terms,
        Stage::Hit,
        ResolutionMode::Operator,
        commitment(&hit_template)?,
    )?;
    let coin = coin_from(&targ_tx, 0);
    let input = input_from_artifact(&target_output, &coin)?;
    let psbt = attach_inputs(
        hit_template.tx.clone(),
        coin,
        input,
        sponsor(2_000, tag + 2),
    )?;
    let intent = prepare_depot_spend(
        &target_output,
        requirement,
        psbt,
        "depots/hit/v1",
        hit_evidence(ResolutionMode::Operator, Some((TARGET, tasig)), tuple),
    )?;
    let hit_tx = complete_candidate(&target_output, &intent, oracle)?;
    assert_eq!(hit_tx.output.len(), 2);
    assert!(hit_tx
        .output
        .iter()
        .all(|output| output.value.to_sat() == DEPOT_SATS / 2));
    Ok((hit_tx, (TARGET, tasig)))
}

#[test]
fn a_single_hit_resolves_target_hit_claim_and_settlement() -> Result<()> {
    let terms = terms();
    let oracle = oracle();
    let alice = Purchase::new(1, 0, TARGET);
    let alice_tuple = purchase_channel(&terms, &alice)?;

    let (hit_tx, target_proof) = resolve_to_hit(&terms, &oracle, &alice_tuple, 10)?;

    // The insurance half is exactly the compiled insurance output.
    let insurance = compile_hit_insurance_burn(&terms, ResolutionMode::Operator)?;
    assert_eq!(
        hit_tx.output[0].script_pubkey,
        bitcoin::ScriptBuf::from(&insurance.address)
    );

    // Alice binds the exact claim transaction the operator blind-signed.
    let hit_claim = compile_hit_claim(&terms, ResolutionMode::Operator, alice.user)?;
    assert_eq!(
        hit_tx.output[1].script_pubkey,
        bitcoin::ScriptBuf::from(&hit_claim.address)
    );
    let clm_template = only_candidate(&hit_claim)?.clone();
    // The purchase-time commitment equals the runtime claim candidate.
    assert_eq!(commitment(&clm_template)?.to_byte_array(), alice_tuple.clm);
    let requirement = depot_requirement(
        &hit_claim,
        &terms,
        Stage::Claim,
        ResolutionMode::Operator,
        zeros(),
    )?;
    let coin = coin_from(&hit_tx, 1);
    let input = input_from_artifact(&hit_claim, &coin)?;
    let psbt = attach_inputs(clm_template.tx.clone(), coin, input, sponsor(2_000, 13))?;
    let intent = prepare_depot_spend(
        &hit_claim,
        requirement,
        psbt,
        "depots/claim/v1",
        hit_evidence(ResolutionMode::Operator, Some(target_proof), &alice_tuple),
    )?;
    let clm_tx = complete_candidate(&hit_claim, &intent, &oracle)?;
    // The claim waits out the monitoring window relative to the Hit.
    assert_eq!(clm_tx.input[0].sequence.to_consensus_u32(), u32::from(TSD));

    // The claim funds the channel's settlement layer; the final commitment
    // splits the claimed half by the last agreed balance.
    let balance = ChannelBalance { user_sats: 30_000 };
    let settlement = compile_claim_output(&terms, ResolutionMode::Operator, alice.user, balance)?;
    assert_eq!(
        clm_tx.output[0].script_pubkey,
        bitcoin::ScriptBuf::from(&settlement.address)
    );
    let commitment_template = only_candidate(&settlement)?;
    assert_eq!(
        commitment_template
            .tx
            .output
            .iter()
            .map(|output| output.value.to_sat())
            .collect::<Vec<_>>(),
        vec![30_000, DEPOT_SATS / 2 - 30_000]
    );
    Ok(())
}

#[test]
fn misses_forgeries_and_wrong_claims_are_rejected() -> Result<()> {
    let terms = terms();
    let oracle = oracle();
    let alice = Purchase::new(1, 0, TARGET);
    let alice_tuple = purchase_channel(&terms, &alice)?;
    let bob = Purchase::new(2, 1, 1);
    let bob_tuple = purchase_channel(&terms, &bob)?;
    let tasig = sign_target(&terms, TARGET);

    let target_output = compile_target_output(&terms, ResolutionMode::Operator)?;
    let hit_template = only_candidate(&target_output)?.clone();
    let requirement = depot_requirement(
        &target_output,
        &terms,
        Stage::Hit,
        ResolutionMode::Operator,
        commitment(&hit_template)?,
    )?;
    let coin = coin_for(&target_output, terms.value(), 20);
    let hit_intent = |witness: Vec<u8>| -> Result<_> {
        let input = input_from_artifact(&target_output, &coin)?;
        let psbt = attach_inputs(
            hit_template.tx.clone(),
            coin.clone(),
            input,
            sponsor(2_000, 21),
        )?;
        prepare_depot_spend(
            &target_output,
            requirement.clone(),
            psbt,
            "depots/hit/v1",
            witness,
        )
    };

    // Bob's channel misses the revealed target.
    let miss = hit_intent(hit_evidence(
        ResolutionMode::Operator,
        Some((TARGET, tasig)),
        &bob_tuple,
    ))?;
    assert!(oracle_signs(&target_output, &miss, &oracle).is_err());

    // A target signature for a different value cannot prove this target.
    let wrong_target = hit_intent(hit_evidence(
        ResolutionMode::Operator,
        Some((TARGET, sign_target(&terms, TARGET - 1))),
        &alice_tuple,
    ))?;
    assert!(oracle_signs(&target_output, &wrong_target, &oracle).is_err());

    // A tampered tuple invalidates the operator's blind signature.
    let mut tampered = alice_tuple.clone();
    tampered.salt[0] ^= 1;
    let forged = hit_intent(hit_evidence(
        ResolutionMode::Operator,
        Some((TARGET, tasig)),
        &tampered,
    ))?;
    assert!(oracle_signs(&target_output, &forged, &oracle).is_err());

    // The hit succeeds with Alice's authentic evidence, then Mallory cannot
    // bind her own claim transaction with Alice's guess signature.
    let (hit_tx, target_proof) = resolve_to_hit(&terms, &oracle, &alice_tuple, 25)?;
    let mallory = user_key(9).x_only_public_key().0;
    let mallory_claim = compile_hit_claim(&terms, ResolutionMode::Operator, mallory)?;
    let stolen_template = only_candidate(&mallory_claim)?.clone();
    assert_ne!(
        commitment(&stolen_template)?.to_byte_array(),
        alice_tuple.clm
    );
    let requirement = depot_requirement(
        &mallory_claim,
        &terms,
        Stage::Claim,
        ResolutionMode::Operator,
        zeros(),
    )?;
    let coin = coin_from(&hit_tx, 1);
    let input = input_from_artifact(&mallory_claim, &coin)?;
    let psbt = attach_inputs(stolen_template.tx.clone(), coin, input, sponsor(2_000, 28))?;
    let theft = prepare_depot_spend(
        &mallory_claim,
        requirement,
        psbt,
        "depots/claim/v1",
        hit_evidence(ResolutionMode::Operator, Some(target_proof), &alice_tuple),
    )?;
    assert!(oracle_signs(&mallory_claim, &theft, &oracle).is_err());
    Ok(())
}

#[test]
fn a_double_hit_burns_both_halves() -> Result<()> {
    let terms = terms();
    let oracle = oracle();
    let alice = Purchase::new(1, 0, TARGET);
    let alice_tuple = purchase_channel(&terms, &alice)?;
    let carol = Purchase::new(4, 2, TARGET);
    let carol_tuple = purchase_channel(&terms, &carol)?;

    let (hit_tx, target_proof) = resolve_to_hit(&terms, &oracle, &alice_tuple, 30)?;

    for (compiled, vout, tag) in [
        (
            compile_hit_insurance_burn(&terms, ResolutionMode::Operator)?,
            0,
            33,
        ),
        (
            compile_hit_claim_burn(&terms, ResolutionMode::Operator)?,
            1,
            35,
        ),
    ] {
        let burn_template = only_candidate(&compiled)?.clone();
        let requirement = depot_requirement(
            &compiled,
            &terms,
            Stage::Burn,
            ResolutionMode::Operator,
            commitment(&burn_template)?,
        )?;
        let coin = coin_from(&hit_tx, vout);
        let input = input_from_artifact(&compiled, &coin)?;
        let psbt = attach_inputs(
            burn_template.tx.clone(),
            coin.clone(),
            input,
            sponsor(2_000, tag),
        )?;

        // The same signature twice proves nothing.
        let replayed = prepare_depot_spend(
            &compiled,
            requirement.clone(),
            psbt.clone(),
            "depots/burn/v1",
            burn_evidence(
                ResolutionMode::Operator,
                Some(target_proof),
                &alice_tuple,
                &alice_tuple,
            ),
        )?;
        assert!(oracle_signs(&compiled, &replayed, &oracle).is_err());

        // Two distinct guess signatures for the target destroy the half.
        let intent = prepare_depot_spend(
            &compiled,
            requirement,
            psbt,
            "depots/burn/v1",
            burn_evidence(
                ResolutionMode::Operator,
                Some(target_proof),
                &alice_tuple,
                &carol_tuple,
            ),
        )?;
        let burn_tx = complete_candidate(&compiled, &intent, &oracle)?;
        assert!(burn_tx.output[0].script_pubkey.is_op_return());
        assert_eq!(burn_tx.output[0].value.to_sat(), DEPOT_SATS / 2);
    }
    Ok(())
}

#[test]
fn the_default_branch_resolves_without_the_operator() -> Result<()> {
    let terms = terms();
    let oracle = oracle();
    // Dave's guess of zero hits the default target.
    let dave = Purchase::new(5, 4, 0);
    let dave_tuple = purchase_channel(&terms, &dave)?;

    // Anyone posts Targ' after the grace window; no signatures are needed.
    let funding = compile_funding(&terms, ResolutionMode::Default)?;
    let targ_template = only_candidate(&funding)?.clone();
    let requirement = default_target_requirement(&funding, &targ_template)?;
    let coin = coin_for(&funding, terms.value(), 40);
    let input = input_from_artifact(&funding, &coin)?;
    let psbt = attach_inputs(targ_template.tx.clone(), coin, input, sponsor(2_000, 41))?;
    let intent = prepare_depot_spend(
        &funding,
        requirement,
        psbt,
        "depots/default-target-empty/v1",
        vec![],
    )?;
    let targ_tx = complete_candidate(&funding, &intent, &oracle)?;
    assert_eq!(
        targ_tx.lock_time.to_consensus_u32(),
        EXPIRY + u32::from(TSD)
    );

    // The default branch's Hit needs only a guess of zero.
    let target_output = compile_target_output(&terms, ResolutionMode::Default)?;
    assert_eq!(
        targ_tx.output[0].script_pubkey,
        bitcoin::ScriptBuf::from(&target_output.address)
    );
    let hit_template = only_candidate(&target_output)?.clone();
    let requirement = depot_requirement(
        &target_output,
        &terms,
        Stage::Hit,
        ResolutionMode::Default,
        commitment(&hit_template)?,
    )?;
    let coin = coin_from(&targ_tx, 0);
    let input = input_from_artifact(&target_output, &coin)?;
    let psbt = attach_inputs(
        hit_template.tx.clone(),
        coin.clone(),
        input,
        sponsor(2_000, 42),
    )?;
    // A nonzero guess cannot hit the default target.
    let alice_tuple = purchase_channel(&terms, &Purchase::new(1, 0, TARGET))?;
    let miss = prepare_depot_spend(
        &target_output,
        requirement.clone(),
        psbt.clone(),
        "depots/hit/v1",
        hit_evidence(ResolutionMode::Default, None, &alice_tuple),
    )?;
    assert!(oracle_signs(&target_output, &miss, &oracle).is_err());
    let intent = prepare_depot_spend(
        &target_output,
        requirement,
        psbt,
        "depots/hit/v1",
        hit_evidence(ResolutionMode::Default, None, &dave_tuple),
    )?;
    let hit_tx = complete_candidate(&target_output, &intent, &oracle)?;

    // The claim binds Clm'_Ui, the default-branch hash of Dave's purchase.
    let hit_claim = compile_hit_claim(&terms, ResolutionMode::Default, dave.user)?;
    let clm_template = only_candidate(&hit_claim)?.clone();
    assert_eq!(
        commitment(&clm_template)?.to_byte_array(),
        dave_tuple.clm_prime
    );
    let requirement = depot_requirement(
        &hit_claim,
        &terms,
        Stage::Claim,
        ResolutionMode::Default,
        zeros(),
    )?;
    let coin = coin_from(&hit_tx, 1);
    let input = input_from_artifact(&hit_claim, &coin)?;
    let psbt = attach_inputs(clm_template.tx.clone(), coin, input, sponsor(2_000, 44))?;
    let intent = prepare_depot_spend(
        &hit_claim,
        requirement,
        psbt,
        "depots/claim/v1",
        hit_evidence(ResolutionMode::Default, None, &dave_tuple),
    )?;
    complete_candidate(&hit_claim, &intent, &oracle)?;
    Ok(())
}

#[test]
fn depot_addresses_are_channel_independent() -> Result<()> {
    let terms = terms();
    // Both resolution branches live in one funding address; compiling with
    // different candidates never changes the address.
    let normal = compile_funding(&terms, ResolutionMode::Operator)?;
    let default = compile_funding(&terms, ResolutionMode::Default)?;
    assert_eq!(normal.address, default.address);
    // The two branches' target outputs are distinct contracts.
    let operator_branch = compile_target_output(&terms, ResolutionMode::Operator)?;
    let default_branch = compile_target_output(&terms, ResolutionMode::Default)?;
    assert_ne!(operator_branch.address, default_branch.address);
    // Every covenant leaf is a recorded program requirement.
    assert_eq!(normal.program_requirements()?.len(), 2);
    // Selling more channels never touches the chain: purchases only create
    // signatures, so the funding artifact is untouched by user count.
    for seed in 0..8u8 {
        purchase_channel(&terms, &Purchase::new(seed, u64::from(seed), 1))?;
    }
    assert_eq!(
        compile_funding(&terms, ResolutionMode::Operator)?.address,
        normal.address
    );
    // A depot must split evenly and wait out a real monitoring window.
    assert!(sapio_contrib::contracts::depots::DepotTerms::new(
        operator_key().x_only_public_key().0,
        target_key().x_only_public_key().0,
        guess_key().x_only_public_key().0,
        *terms.oracle_root(),
        Amount::from_sat(DEPOT_SATS + 1),
        PRIME,
        sapio_base::timelocks::AbsHeight::try_from(EXPIRY).unwrap(),
        TSD,
        Amount::from_sat(10_000),
    )
    .is_err());
    Ok(())
}
