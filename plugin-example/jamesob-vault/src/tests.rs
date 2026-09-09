use super::*;
#[test]
fn fee_rounds_up_and_rejects_unrepresentable_total() {
    assert_eq!(estimated_fee(1.into(), 1).unwrap().as_sat(), 1);
    assert_eq!(estimated_fee(250.into(), 100).unwrap().as_sat(), 100);
    assert!(estimated_fee(u64::MAX.into(), u64::MAX).is_err());
}

#[test]
fn vault_records_fees_in_required_funding() {
    use sapio_base::effects::EffectPath;
    use sapio_ctv_emulator_trait::CTVAvailable;
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
            let backup_script = vault.backup_addr.script_pubkey();
            let ctx = Context::new(
                bitcoin::Network::Regtest,
                Amount::from_sat(10000),
                Arc::new(CTVAvailable),
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
                    let output_total = template.tx.output.iter().map(|o| o.value).sum::<u64>();
                    assert_eq!(funding - output_total, expected_fee);
                    assert_eq!(
                        template.max - template.total_amount(),
                        Amount::from_sat(expected_fee)
                    );
                    if cpfp {
                        assert_eq!(template.tx.output[0].value, 500);
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
                            assert!(template
                                .tx
                                .output
                                .last()
                                .unwrap()
                                .script_pubkey
                                .is_v1_p2tr());
                        }
                        label => panic!("Unexpected vault transaction: {label:?}"),
                    }
                    pending.extend(
                        template
                            .outputs
                            .iter()
                            .filter(|output| !output.contract.ctv_to_tx.is_empty())
                            .map(|output| (&output.contract, output.amount.as_sat())),
                    );
                }
            }
            assert_eq!((backups, redeems), (2, 1));
        }
    }
}
