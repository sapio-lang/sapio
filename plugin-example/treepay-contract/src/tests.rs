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
        None,
    )
}
fn address() -> bitcoin::Address {
    "bcrt1qumrrqgt7e3a7damzm8x97m6sjs20u8hjw2hcjj"
        .parse::<bitcoin::Address<bitcoin::address::NetworkUnchecked>>()
        .unwrap()
        .require_network(bitcoin::Network::Regtest)
        .unwrap()
}

fn tree(count: usize, radix: usize) -> TreePay {
    TreePay {
        participants: (0..count)
            .map(|_| Payment {
                amount: Amount::from_sat(1000),
                address: address().into_unchecked(),
            })
            .collect(),
        radix,
        fee_sats_per_tx: Amount::from_sat(100),
        timelock_backpressure: None,
    }
}
#[test]
fn invalid_trees_fail_before_queue_expansion() {
    for radix in [0, 1] {
        assert!(tree(2, radix).validate().is_err());
        assert!(tree(2, radix).compile(context(2200)).is_err());
    }
    assert!(tree(0, 2).validate().is_err());
    let mut overflowing = tree(2, 2);
    overflowing.participants[0].amount = Amount::from_sat(u64::MAX);
    assert!(overflowing.validate().is_err());
    assert!(TreePay::try_from(BatchingTraitVersion0_1_1 {
        payments: tree(1, 2).participants,
        feerate_per_byte: Amount::from_sat(u64::MAX),
    })
    .is_err());
}
#[test]
fn uneven_tree_preserves_every_payment_and_transaction_fee() {
    fn inspect(object: &Compiled) -> (usize, u64) {
        let Some(template) = object.ctv_to_tx.values().next() else {
            return (1, 0);
        };
        assert!(template.tx.output.len() <= 3);
        let mut result = (0, 100);
        for output in &template.outputs {
            let child = inspect(&output.contract);
            result.0 += child.0;
            result.1 += child.1;
            if child.1 == 0 {
                assert_eq!(output.amount, Amount::from_sat(1000));
            }
        }
        result
    }
    let tree = tree(11, 3);
    assert_eq!(tree.validate().unwrap().to_sat(), 11500);
    let compiled = tree.compile(context(11500)).unwrap();
    compiled.validate().unwrap();
    assert_eq!(inspect(&compiled), (11, 500));
    assert!(tree.compile(context(11499)).is_err());
}
