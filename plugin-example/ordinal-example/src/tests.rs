use super::*;
use sapio::contract::Compilable;
use sapio_base::covenant::LoweringPlan;
use sapio_base::effects::EffectPath;
use std::sync::Arc;
fn context(amount: u64) -> Context {
    Context::new(
        bitcoin::Network::Regtest,
        Amount::from_sat(amount),
        LoweringPlan::Native,
        EffectPath::try_from("example").unwrap(),
        Arc::new(Default::default()),
        Some(sapio_base::plugin_args::OrdinalsInfo(vec![(
            Ordinal(100),
            Ordinal(100 + amount),
        )])),
    )
}
fn address() -> bitcoin::Address {
    "bcrt1qumrrqgt7e3a7damzm8x97m6sjs20u8hjw2hcjj"
        .parse::<bitcoin::Address<bitcoin::address::NetworkUnchecked>>()
        .unwrap()
        .require_network(bitcoin::Network::Regtest)
        .unwrap()
}
fn key(byte: u8) -> bitcoin::XOnlyPublicKey {
    use bitcoin::secp256k1::{Keypair, SecretKey};
    Keypair::from_secret_key(
        &bitcoin::secp256k1::Secp256k1::new(),
        &SecretKey::from_slice(&[byte; 32]).unwrap(),
    )
    .x_only_public_key()
    .0
}

fn ordinal(n: u64) -> SimpleOrdinal {
    SimpleOrdinal {
        ordinal: n,
        owner: key(1),
    }
}
fn sale() -> Sell {
    Sell {
        purchaser: address().into_unchecked(),
        amount: Amount::from_sat(1000).into(),
        change: Amount::from_sat(200).into(),
        fee: Amount::from_sat(100).into(),
    }
}
#[test]
fn sale_places_exact_target_sat_at_start_of_buyer_output() {
    let template = ordinal(110).continue_sell(context(1000), sale()).unwrap();
    assert_eq!(template.tx.input.len(), 2);
    assert_eq!(
        template
            .tx
            .output
            .iter()
            .map(|o| o.value.to_sat())
            .collect::<Vec<_>>(),
        vec![10, 501, 489, 1000, 200]
    );
    assert_eq!(
        template.tx.output[1].script_pubkey,
        address().script_pubkey()
    );
    assert_eq!(
        template.tx.output[4].script_pubkey,
        address().script_pubkey()
    );
    assert_eq!(template.max, Amount::from_sat(2300));
    let funding = template.funding_constraints.as_ref().unwrap();
    assert_eq!(funding.maximum_fee, Amount::from_sat(100));
    assert_eq!(funding.inputs[1].name, "buyer_funding");
    assert_eq!(funding.inputs[1].minimum, Amount::from_sat(1300));
    assert_eq!(
        funding.outputs,
        [
            "owner_prefix",
            "ordinal",
            "owner_remainder",
            "price",
            "buyer_change"
        ]
    );
    assert_eq!(
        ordinal(100)
            .continue_sell(context(1000), sale())
            .unwrap()
            .tx
            .output[0]
            .value
            .to_sat(),
        501
    );
}
#[test]
fn missing_ordinal_and_insufficient_padding_are_rejected() {
    for n in [99, 600, 1100] {
        assert!(ordinal(n).compile(context(1000)).is_err());
        assert!(ordinal(n).continue_sell(context(1000), sale()).is_err());
    }
    assert!(ordinal(100).compile(context(500)).is_err());
    let mut overflow = sale();
    overflow.amount = Amount::from_sat(u64::MAX).into();
    assert!(ordinal(100).continue_sell(context(1000), overflow).is_err());
}

#[test]
fn planner_sale_preserves_target_owner_balance_buyer_change_and_fees() {
    let template = ordinal(110)
        .continue_sell_with_planner(context(1000), sale())
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(template.tx.output[0].value.to_sat(), 10);
    assert_eq!(template.tx.output[1].value.to_sat(), 501);
    assert_eq!(
        template.tx.output[1].script_pubkey,
        address().script_pubkey()
    );
    assert_eq!(
        template
            .tx
            .output
            .iter()
            .filter(|o| o.script_pubkey == address().script_pubkey())
            .map(|o| o.value.to_sat())
            .sum::<u64>(),
        701
    );
    assert_eq!(
        template
            .tx
            .output
            .iter()
            .map(|o| o.value.to_sat())
            .sum::<u64>(),
        2200
    );
    assert_eq!(template.max, Amount::from_sat(2300));
    assert_eq!(template.required_input_amount, Amount::from_sat(1000));
    let mut no_change = sale();
    no_change.change = Amount::ZERO.into();
    assert!(ordinal(110)
        .continue_sell_with_planner(context(1000), no_change)
        .is_ok());
}
