use bitcoin::hashes::Hash;
use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
use bitcoin::{psbt::Psbt, Address, Amount, FeeRate, Network, OutPoint, TxOut, Witness};
use sapio::contract::{Compiled, Context};
use sapio::template::{funding::FundingError, OutputAmount, Surplus, Template};
use sapio_base::LoweringPlan;
use std::sync::Arc;

fn template(rate: Option<FeeRate>) -> Template {
    let key =
        Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[8; 32]).unwrap());
    let recipient = Compiled::from_address(
        Address::p2tr(
            &Secp256k1::new(),
            key.x_only_public_key().0,
            None,
            Network::Regtest,
        ),
        Amount::ZERO,
    );
    let context = Context::new(
        Network::Regtest,
        Amount::from_sat(700),
        LoweringPlan::Native,
        "funding".try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    );
    let mut plan = context.template_plan();
    plan.input("sponsor", Amount::from_sat(300)).unwrap();
    plan.output(
        "recipient",
        OutputAmount::Exact(Amount::from_sat(900)),
        &recipient,
    )
    .unwrap();
    plan.reserve_fees(Amount::from_sat(100));
    plan.surplus(Surplus::Fees {
        maximum: Amount::from_sat(250),
    });
    if let Some(rate) = rate {
        plan.require_feerate(rate);
    }
    plan.finish().unwrap()
}

fn funded(template: &Template, values: [u64; 2]) -> Psbt {
    let mut tx = template.tx.clone();
    for (index, input) in tx.input.iter_mut().enumerate() {
        input.previous_output =
            OutPoint::new(bitcoin::Txid::from_byte_array([index as u8 + 1; 32]), 0);
    }
    let mut psbt = Psbt::from_unsigned_tx(tx).unwrap();
    for (input, value) in psbt.inputs.iter_mut().zip(values) {
        input.witness_utxo = Some(TxOut {
            value: Amount::from_sat(value),
            script_pubkey: template.tx.output[0].script_pubkey.clone(),
        });
    }
    psbt
}

#[test]
fn named_contributions_and_fee_cap_survive_serialization() {
    let template: Template =
        serde_json::from_value(serde_json::to_value(template(None)).unwrap()).unwrap();
    // Enough total money cannot hide a sponsor failing its declared contribution.
    assert!(
        matches!(template.check_funded_psbt(&funded(&template, [800, 200])),
        Err(FundingError::UnderfundedInput { index: 1, name, .. }) if name == "sponsor")
    );
    assert_eq!(
        template
            .check_funded_psbt(&funded(&template, [700, 300]))
            .unwrap()
            .actual_fee_sats,
        Some(100)
    );
    assert_eq!(
        template
            .check_funded_psbt(&funded(&template, [700, 450]))
            .unwrap()
            .actual_fee_sats,
        Some(250)
    );
    assert!(matches!(
        template.check_funded_psbt(&funded(&template, [700, 451])),
        Err(FundingError::ExcessiveFee { .. })
    ));
}

#[test]
fn partial_funding_stays_unknown_but_cannot_hide_already_excessive_fees() {
    let template = template(None);
    let mut psbt = funded(&template, [700, 300]);
    psbt.inputs[1].witness_utxo = None;
    let report = template.check_funded_psbt(&psbt).unwrap();
    assert_eq!(report.missing_inputs, [1]);
    assert_eq!(report.actual_fee_sats, None);
    assert_eq!(report.known_input_sats, 700);
    psbt.inputs[0].witness_utxo.as_mut().unwrap().value = Amount::from_sat(1_151);
    assert!(matches!(
        template.check_funded_psbt(&psbt),
        Err(FundingError::ExcessiveFee { .. })
    ));
}

#[test]
fn fee_rate_is_residual_until_final_witness_weight_is_observable() {
    let template = template(Some(FeeRate::from_sat_per_kwu(250)));
    let mut psbt = funded(&template, [700, 450]);
    let report = template.check_funded_psbt(&psbt).unwrap();
    assert!(report.fee_rate_pending);
    assert_eq!(report.observed_weight_wu, None);
    // This API measures supplied final fields; signature validity is separate.
    for input in &mut psbt.inputs {
        input.final_script_witness = Some(Witness::from_slice(&[vec![0; 64]]));
    }
    let report = template.check_funded_psbt(&psbt).unwrap();
    assert!(!report.fee_rate_pending);
    assert!(report.observed_weight_wu.unwrap() > 0);
    psbt.inputs[1].final_script_witness = Some(Witness::from_slice(&[vec![0; 2_000]]));
    assert!(matches!(
        template.check_funded_psbt(&psbt),
        Err(FundingError::InsufficientFeeRate { .. })
    ));
}

#[test]
fn changed_commitments_and_invalid_constraint_declarations_are_rejected() {
    let template = template(None);
    let mut psbt = funded(&template, [700, 300]);
    psbt.unsigned_tx.output[0].value = Amount::from_sat(899);
    assert!(matches!(
        template.check_funded_psbt(&psbt),
        Err(FundingError::TransactionMismatch)
    ));
    let mut tampered = template.clone();
    tampered.funding_constraints.as_mut().unwrap().inputs[1].minimum = Amount::from_sat(299);
    assert!(matches!(
        tampered.validate_funding_constraints(),
        Err(FundingError::InvalidConstraints(_))
    ));
    let mut tampered = template.clone();
    tampered.funding_constraints.as_mut().unwrap().inputs[1].name = "contract".into();
    assert!(matches!(
        tampered.validate_funding_constraints(),
        Err(FundingError::InvalidConstraints(_))
    ));
    let mut tampered = template.clone();
    tampered.funding_constraints.as_mut().unwrap().maximum_fee = Amount::from_sat(99);
    assert!(matches!(
        tampered.validate_funding_constraints(),
        Err(FundingError::InvalidConstraints(_))
    ));
}

#[test]
fn funding_schema_requires_the_field_while_preserving_explicit_null() {
    let schema = serde_json::to_value(schemars::schema_for!(Template)).unwrap();
    assert!(schema["required"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("funding_constraints")));
    assert!(schema["properties"]["funding_constraints"]["anyOf"]
        .as_array()
        .unwrap()
        .iter()
        .any(|alternative| alternative["type"] == "null"));

    let mut serialized = serde_json::to_value(template(None)).unwrap();
    assert!(serde_json::from_value::<Template>(serialized.clone())
        .unwrap()
        .funding_constraints
        .is_some());
    serialized["funding_constraints"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<Template>(serialized.clone())
        .unwrap()
        .funding_constraints
        .is_none());
    serialized
        .as_object_mut()
        .unwrap()
        .remove("funding_constraints");
    assert!(serde_json::from_value::<Template>(serialized).is_err());
}

#[test]
fn inconsistent_previous_outputs_fail_before_fee_arithmetic() {
    let template = template(None);
    let mut psbt = funded(&template, [700, 300]);
    psbt.unsigned_tx.input[1].previous_output = psbt.unsigned_tx.input[0].previous_output;
    assert!(matches!(
        template.check_funded_psbt(&psbt),
        Err(FundingError::Evidence(
            sapio_base::psbt::FundingError::DuplicateInput(1)
        ))
    ));
    let mut psbt = funded(&template, [700, 300]);
    psbt.inputs.pop();
    assert!(matches!(
        template.check_funded_psbt(&psbt),
        Err(FundingError::Evidence(
            sapio_base::psbt::FundingError::Structure(_)
        ))
    ));
    let mut psbt = funded(&template, [700, 300]);
    psbt.inputs[0].non_witness_utxo = Some(psbt.unsigned_tx.clone());
    assert!(matches!(
        template.check_funded_psbt(&psbt),
        Err(FundingError::Evidence(
            sapio_base::psbt::FundingError::PreviousTransaction(0)
        ))
    ));
}

#[test]
fn fee_caps_below_the_unsigned_weight_are_rejected_before_witnesses_exist() {
    let mut template = template(None);
    let constraints = template.funding_constraints.as_mut().unwrap();
    constraints.minimum_feerate = Some(FeeRate::from_sat_per_vb(100).unwrap());
    let restored: Template =
        serde_json::from_value(serde_json::to_value(&template).unwrap()).unwrap();
    assert!(
        matches!(restored.validate_funding_constraints(), Err(FundingError::InvalidConstraints(reason)) if reason.contains("unsigned transaction"))
    );
    assert!(matches!(
        restored.check_funded_psbt(&funded(&restored, [700, 450])),
        Err(FundingError::InvalidConstraints(_))
    ));

    let context = Context::new(
        Network::Regtest,
        Amount::from_sat(1000),
        LoweringPlan::Native,
        "impossible_fees".try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    );
    let recipient = Compiled::from_op_return(b"fee constraint").unwrap();
    let mut plan = context.template_plan();
    plan.output(
        "recipient",
        OutputAmount::Exact(Amount::from_sat(1000)),
        &recipient,
    )
    .unwrap();
    plan.require_feerate(FeeRate::from_sat_per_vb(1).unwrap());
    assert!(matches!(
        plan.finish(),
        Err(sapio::template::PlanError::Funding(
            FundingError::InvalidConstraints(_)
        ))
    ));
}
