use super::*;
use sapio_base::covenant::LoweringPlan;
#[test]
fn fee_rounds_up_and_rejects_unrepresentable_total() {
    assert_eq!(estimated_fee(1.into(), 1).unwrap().to_sat(), 1);
    assert_eq!(estimated_fee(250.into(), 100).unwrap().to_sat(), 100);
    assert!(estimated_fee(u64::MAX.into(), u64::MAX).is_err());
}

fn compile_withdrawal<S: State>(
    action: &str,
    amount: u64,
    rate: u64,
    cpfp: bool,
) -> Result<Compiled, CompilationError> {
    use sapio_base::effects::EffectPath;
    use std::sync::Arc;
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../contrib/vectors/examples/jamesob-vault.json"
    ))
    .unwrap();
    let mut vault: Vault<S> = serde_json::from_value(fixture["arguments"].clone()).unwrap();
    vault.default_feerate = rate.into();
    if cpfp {
        vault.cpfp = Some(Output {
            address: vault.backup_addr.clone(),
            amount: Amount::from_sat(500).into(),
        });
    }
    // Use a separate destination: an exact cold withdrawal to backup_addr
    // would duplicate the already-committed backup transaction.
    let destination = bitcoin::Address::p2tr(
        &bitcoin::secp256k1::Secp256k1::verification_only(),
        vault.hot_key.to_x_only_pub(),
        None,
        bitcoin::Network::Regtest,
    );
    let effects = serde_json::from_value(serde_json::json!({
        "effects": {
            format!("vault/@action/{action}/@suggested"): {
                "withdrawal": {
                    "address": destination,
                    "amount": AmountF64::from(Amount::from_sat(amount)),
                }
            }
        }
    }))
    .unwrap();
    vault.compile(Context::new(
        bitcoin::Network::Regtest,
        Amount::from_sat(10_000),
        LoweringPlan::Native,
        EffectPath::try_from("vault").unwrap(),
        Arc::new(effects),
        None,
    ))
}

fn assert_withdrawal<S: State>(action: &str) {
    use sapio::template::funding::FundingError;
    for rate in [250, 251] {
        for cpfp in [false, true] {
            let object = compile_withdrawal::<S>(action, 5000, rate, cpfp).unwrap();
            object.validate().unwrap();
            assert_eq!(object.suggested_txs.len(), 1);
            let template = object.suggested_txs.values().next().unwrap();
            let unsigned_size = bitcoin::consensus::serialize(&template.tx).len() as u64;
            let fee = (unsigned_size * 4 * rate).div_ceil(1000);
            assert_eq!(template.max.to_sat(), 10_000);
            assert_eq!(template.maximum_fee, Some(Amount::from_sat(fee)));
            assert_eq!(template.tx.output.len(), 2);
            assert_eq!(template.tx.output[0].value.to_sat(), 5000);
            assert_eq!(template.tx.output[1].value.to_sat(), 5000 - fee);
            assert_eq!(
                template.tx.input[0].sequence,
                if action == "spend_hot" {
                    bitcoin::Sequence::from_height(6)
                } else {
                    bitcoin::Sequence::from_512_second_intervals(0)
                }
            );
            // Change is again protected by both secure-vault committed paths,
            // including a fresh delay before the hot key can spend it.
            let change = &template.outputs[1].contract;
            assert!(template.tx.output[1].script_pubkey.is_p2tr());
            assert_eq!(change.ctv_to_tx.len(), 2);
            assert!(change.suggested_txs.is_empty());
            assert!(change.ctv_to_tx.values().any(|template| {
                template.metadata_map_s2s.label.as_deref() == Some("begin redeem")
            }));
            assert_eq!(
                template
                    .check_funding_amounts(&[Some(Amount::from_sat(10_000))])
                    .unwrap()
                    .actual_fee_sats,
                Some(fee)
            );
            assert!(matches!(
                template.check_funding_amounts(&[Some(Amount::from_sat(10_001))]),
                Err(FundingError::ExcessiveFee { .. })
            ));
            assert!(matches!(
                template.check_funding_amounts(&[Some(Amount::from_sat(9999))]),
                Err(FundingError::InsufficientFunding { .. })
            ));

            let mut no_change_tx = template.tx.clone();
            no_change_tx.output.pop();
            let no_change_fee =
                (bitcoin::consensus::serialize(&no_change_tx).len() as u64 * 4 * rate)
                    .div_ceil(1000);
            let exact =
                compile_withdrawal::<S>(action, 10_000 - no_change_fee, rate, cpfp).unwrap();
            let exact = exact.suggested_txs.values().next().unwrap();
            assert_eq!(exact.tx.output.len(), 1);
            assert_eq!(exact.maximum_fee, Some(Amount::from_sat(no_change_fee)));
            for too_large in [10_000, 10_000 - no_change_fee + 1] {
                assert!(compile_withdrawal::<S>(action, too_large, rate, cpfp).is_err());
            }
            // Do not silently donate change too small to pay for its output.
            assert!(
                compile_withdrawal::<S>(action, 10_000 - no_change_fee - 1, rate, cpfp).is_err()
            );
        }
    }
}

#[test]
fn hot_withdrawals_reserve_fees_and_return_secure_change() {
    assert_withdrawal::<Redeeming>("spend_hot");
}

#[test]
fn cold_withdrawals_reserve_fees_and_return_secure_change() {
    assert_withdrawal::<Secure>("spend_cold");
    assert_withdrawal::<Redeeming>("spend_cold");
}

#[test]
fn vault_records_fees_in_required_funding() {
    use sapio_base::effects::EffectPath;
    use std::sync::Arc;
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../contrib/vectors/examples/jamesob-vault.json"
    ))
    .unwrap();
    for rate in [250, 251] {
        for cpfp in [false, true] {
            let mut vault: Vault<Secure> =
                serde_json::from_value(fixture["arguments"].clone()).unwrap();
            vault.default_feerate = rate.into();
            if cpfp {
                vault.cpfp = Some(Output {
                    address: vault.backup_addr.clone(),
                    amount: Amount::from_sat(500).into(),
                });
            }
            let backup_script = vault
                .backup_addr
                .clone()
                .require_network(bitcoin::Network::Regtest)
                .unwrap()
                .script_pubkey();
            let ctx = Context::new(
                bitcoin::Network::Regtest,
                Amount::from_sat(10000),
                LoweringPlan::Native,
                EffectPath::try_from("vault").unwrap(),
                Arc::new(Default::default()),
                None,
            );
            let compiled = vault.compile(ctx).unwrap();
            compiled.validate().unwrap();
            assert_eq!(compiled.ctv_to_tx.len(), 2);
            assert_eq!(compiled.required_input_amount, Amount::from_sat(10000));

            let mut pending = vec![(&compiled, 10000)];
            let mut backups = 0;
            let mut redeems = 0;
            while let Some((object, funding)) = pending.pop() {
                for template in object.ctv_to_tx.values() {
                    assert_eq!(template.max, Amount::from_sat(funding));
                    assert_eq!(template.required_input_amount, Amount::from_sat(funding));
                    assert_eq!(template.tx.output.len(), if cpfp { 2 } else { 1 });
                    assert_eq!(template.tx.input.len(), 1);
                    assert!(template.tx.input[0].script_sig.is_empty());
                    assert!(template.tx.input[0].witness.is_empty());

                    let unsigned_bytes = bitcoin::consensus::serialize(&template.tx).len() as u64;
                    let expected_fee = (unsigned_bytes * 4 * rate).div_ceil(1000);
                    let output_total = template
                        .tx
                        .output
                        .iter()
                        .map(|o| o.value.to_sat())
                        .sum::<u64>();
                    assert_eq!(funding - output_total, expected_fee);
                    assert_eq!(
                        template.max - template.total_amount(),
                        Amount::from_sat(expected_fee)
                    );
                    if cpfp {
                        assert_eq!(template.tx.output[0].value.to_sat(), 500);
                        assert_eq!(template.tx.output[0].script_pubkey, backup_script);
                    }
                    match template.metadata_map_s2s.label.as_deref() {
                        Some("backup to cold") => {
                            backups += 1;
                            assert_eq!(
                                template.tx.output.last().unwrap().script_pubkey,
                                backup_script
                            );
                        }
                        Some("begin redeem") => {
                            redeems += 1;
                            assert!(template.tx.output.last().unwrap().script_pubkey.is_p2tr());
                        }
                        label => panic!("Unexpected vault transaction: {label:?}"),
                    }
                    pending.extend(
                        template
                            .outputs
                            .iter()
                            .filter(|output| !output.contract.ctv_to_tx.is_empty())
                            .map(|output| (&output.contract, output.amount.to_sat())),
                    );
                }
            }
            assert_eq!((backups, redeems), (2, 1));
        }
    }
}
