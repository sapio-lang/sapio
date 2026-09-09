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
    let vault: Vault<Secure> = serde_json::from_value(fixture["arguments"].clone()).unwrap();
    let ctx = Context::new(
        bitcoin::Network::Regtest,
        Amount::from_sat(10000),
        Arc::new(CTVAvailable),
        EffectPath::try_from("vault").unwrap(),
        Arc::new(Default::default()),
        None,
    );
    let compiled = vault.compile(ctx).unwrap();
    assert_eq!(compiled.ctv_to_tx.len(), 2);
    assert_eq!(compiled.required_input_amount, Amount::from_sat(10000));
    for template in compiled.ctv_to_tx.values() {
        assert_eq!(template.max, Amount::from_sat(10000));
        assert!(template.tx.output.iter().map(|o| o.value).sum::<u64>() < 10000);
    }
}
