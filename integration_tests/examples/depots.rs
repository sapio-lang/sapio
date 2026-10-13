//! Resolve a disposable depot end-to-end without broadcasting: an operator
//! target reveal, a hit, the blind-signed claim binding, a double-hit burn,
//! and the operator-free default branch.

use bitcoin::consensus::encode::serialize_hex;
use bitcoin::hashes::{sha256, Hash};
use sapio_contrib::contracts::depots::{target_evidence, ChannelBalance, ResolutionMode, Stage};
use sapio_integration_tests::depots_example::*;
use serde_json::json;

const TARGET: u64 = 3;

fn main() -> Result<(), Error> {
    let terms = terms();
    let oracle = oracle();

    // Selling channels is pure signing: Alice guesses the eventual target,
    // Bob does not, and Carol collides with Alice. Nothing touches the chain.
    let alice = Purchase::new(1, 0, TARGET);
    let bob = Purchase::new(2, 1, 1);
    let carol = Purchase::new(4, 2, TARGET);
    let alice_tuple = purchase_channel(&terms, &alice)?;
    let bob_tuple = purchase_channel(&terms, &bob)?;
    let carol_tuple = purchase_channel(&terms, &carol)?;

    let funding = compile_funding(&terms, ResolutionMode::Operator)?;
    println!(
        "{}",
        json!({
            "depot_address": funding.address,
            "value_sats": DEPOT_SATS,
            "prime": PRIME,
            "expiry": EXPIRY,
            "to_self_delay": TSD,
            "channels_sold": 3,
        })
    );

    // After expiry the operator reveals t by signing it; the oracle enforces
    // the fixed Target transaction and the target's range.
    let tasig = sign_target(&terms, TARGET);
    let targ_template = only_candidate(&funding)?.clone();
    let requirement = depot_requirement(
        &funding,
        &terms,
        Stage::TargetReveal,
        ResolutionMode::Operator,
        commitment(&targ_template)?,
    )?;
    let coin = coin_for(&funding, terms.value(), 1);
    let input = input_from_artifact(&funding, &coin)?;
    let psbt = attach_inputs(targ_template.tx.clone(), coin, input, sponsor(2_000, 2))?;
    let intent = prepare_depot_spend(
        &funding,
        requirement,
        psbt,
        "depots/target/v1",
        target_evidence(TARGET, &tasig),
    )?;
    let targ_tx = complete_candidate(&funding, &intent, &oracle)?;
    println!(
        "{}",
        json!({"target_revealed": TARGET, "targ_txid": targ_tx.compute_txid()})
    );

    // Alice's channel hits; her evidence forces the fixed 50/50 split.
    let target_output = compile_target_output(&terms, ResolutionMode::Operator)?;
    let hit_template = only_candidate(&target_output)?.clone();
    let requirement = depot_requirement(
        &target_output,
        &terms,
        Stage::Hit,
        ResolutionMode::Operator,
        commitment(&hit_template)?,
    )?;
    let coin = coin_from(&targ_tx, 0);
    let input = input_from_artifact(&target_output, &coin)?;
    let psbt = attach_inputs(
        hit_template.tx.clone(),
        coin.clone(),
        input,
        sponsor(2_000, 3),
    )?;
    // Bob's channel misses: the oracle refuses his guess signature for t=3.
    let miss = prepare_depot_spend(
        &target_output,
        requirement.clone(),
        psbt.clone(),
        "depots/hit/v1",
        hit_evidence(ResolutionMode::Operator, Some((TARGET, tasig)), &bob_tuple),
    )?;
    assert!(oracle_signs(&target_output, &miss, &oracle).is_err());
    let intent = prepare_depot_spend(
        &target_output,
        requirement,
        psbt,
        "depots/hit/v1",
        hit_evidence(
            ResolutionMode::Operator,
            Some((TARGET, tasig)),
            &alice_tuple,
        ),
    )?;
    let hit_tx = complete_candidate(&target_output, &intent, &oracle)?;
    println!(
        "{}",
        json!({"hit_txid": hit_tx.compute_txid(), "halves_sats": DEPOT_SATS / 2, "miss_rejected_for": "bob"})
    );

    // Alice binds the exact claim transaction the operator blind-signed at
    // purchase: a CTV whose hash arrives in evidence, authenticated by CSFS.
    let hit_claim = compile_hit_claim(&terms, ResolutionMode::Operator, alice.user)?;
    let clm_template = only_candidate(&hit_claim)?.clone();
    assert_eq!(commitment(&clm_template)?.to_byte_array(), alice_tuple.clm);
    let requirement = depot_requirement(
        &hit_claim,
        &terms,
        Stage::Claim,
        ResolutionMode::Operator,
        sha256::Hash::from_byte_array([0u8; 32]),
    )?;
    let coin = coin_from(&hit_tx, 1);
    let input = input_from_artifact(&hit_claim, &coin)?;
    let psbt = attach_inputs(clm_template.tx.clone(), coin, input, sponsor(2_000, 4))?;
    let intent = prepare_depot_spend(
        &hit_claim,
        requirement,
        psbt,
        "depots/claim/v1",
        hit_evidence(
            ResolutionMode::Operator,
            Some((TARGET, tasig)),
            &alice_tuple,
        ),
    )?;
    let clm_tx = complete_candidate(&hit_claim, &intent, &oracle)?;
    let settlement = compile_claim_output(
        &terms,
        ResolutionMode::Operator,
        alice.user,
        ChannelBalance { user_sats: 30_000 },
    )?;
    println!(
        "{}",
        json!({
            "claim_txid": clm_tx.compute_txid(),
            "claim_hex": serialize_hex(&clm_tx),
            "settlement_address": settlement.address,
        })
    );

    // Carol also hit t=3: her second guess signature burns the insurance
    // half — the griefing penalty that makes multi-hits and operator
    // equivocation unprofitable.
    let insurance = compile_hit_insurance_burn(&terms, ResolutionMode::Operator)?;
    let burn_template = only_candidate(&insurance)?.clone();
    let requirement = depot_requirement(
        &insurance,
        &terms,
        Stage::Burn,
        ResolutionMode::Operator,
        commitment(&burn_template)?,
    )?;
    let coin = coin_from(&hit_tx, 0);
    let input = input_from_artifact(&insurance, &coin)?;
    let psbt = attach_inputs(burn_template.tx.clone(), coin, input, sponsor(2_000, 5))?;
    let intent = prepare_depot_spend(
        &insurance,
        requirement,
        psbt,
        "depots/burn/v1",
        burn_evidence(
            ResolutionMode::Operator,
            Some((TARGET, tasig)),
            &alice_tuple,
            &carol_tuple,
        ),
    )?;
    let burn_tx = complete_candidate(&insurance, &intent, &oracle)?;
    println!(
        "{}",
        json!({"double_hit_burn_txid": burn_tx.compute_txid(), "burned_sats": DEPOT_SATS / 2})
    );

    // Had the operator withheld the target, anyone could resolve the depot
    // along the default branch after the grace window, with target zero.
    let funding_default = compile_funding(&terms, ResolutionMode::Default)?;
    let default_template = only_candidate(&funding_default)?.clone();
    let requirement = default_target_requirement(&funding_default, &default_template)?;
    let coin = coin_for(&funding_default, terms.value(), 6);
    let input = input_from_artifact(&funding_default, &coin)?;
    let psbt = attach_inputs(default_template.tx.clone(), coin, input, sponsor(2_000, 7))?;
    let intent = prepare_depot_spend(
        &funding_default,
        requirement,
        psbt,
        "depots/default-target-empty/v1",
        vec![],
    )?;
    let default_tx = complete_candidate(&funding_default, &intent, &oracle)?;
    println!(
        "{}",
        json!({
            "default_targ_txid": default_tx.compute_txid(),
            "valid_from_height": EXPIRY + u32::from(TSD),
            "operator_signatures_needed": 0,
        })
    );
    Ok(())
}
