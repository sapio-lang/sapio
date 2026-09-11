use super::*;
use bitcoin::secp256k1::{Keypair, SecretKey};
use bitcoin::{Network, ScriptBuf};
use sapio_base::covenant::LoweringPlan;
use sapio_base::effects::EffectPath;
use std::collections::BTreeMap;
use std::sync::Arc;

fn keys(count: u8) -> Vec<XOnlyPublicKey> {
    (1..=count)
        .map(|byte| {
            Keypair::from_secret_key(
                &Secp256k1::new(),
                &SecretKey::from_slice(&[byte; 32]).unwrap(),
            )
            .x_only_public_key()
            .0
        })
        .collect()
}

fn context(sats: u64) -> Context {
    Context::new(
        Network::Regtest,
        Amount::from_sat(sats),
        LoweringPlan::Native,
        EffectPath::try_from("mining").unwrap(),
        Arc::new(Default::default()),
        None,
    )
}

fn inspect(
    compiled: &Compiled,
    radix: usize,
    fee: u64,
    leaves: &mut BTreeMap<ScriptBuf, u64>,
) -> u64 {
    let mut transactions = 0;
    for template in compiled.ctv_to_tx.values() {
        transactions += 1;
        assert!(!template.tx.output.is_empty());
        assert!(template.tx.output.len() <= radix);
        assert_eq!(
            template.max.to_sat()
                - template
                    .tx
                    .output
                    .iter()
                    .map(|o| o.value.to_sat())
                    .sum::<u64>(),
            fee
        );
        for (output, metadata) in template.tx.output.iter().zip(&template.outputs) {
            if metadata.contract.ctv_to_tx.is_empty() {
                assert!(leaves
                    .insert(output.script_pubkey.clone(), output.value.to_sat())
                    .is_none());
            } else {
                transactions += inspect(&metadata.contract, radix, fee, leaves);
            }
        }
    }
    transactions
}

#[test]
fn reward_tree_pays_each_miner_once_and_reserves_every_fee() {
    for (miners, radix) in [(1, 4), (5, 4), (9, 2)] {
        let payout = MiningPayout::from_reward(
            keys(miners),
            Amount::from_sat(50_003),
            radix,
            Amount::from_sat(100),
        )
        .unwrap();
        assert_eq!(payout.funding_required().unwrap().to_sat(), 50_003);
        let compiled = payout.compile(context(50_003)).unwrap();
        compiled.validate().unwrap();
        let mut leaves = BTreeMap::new();
        let transactions = inspect(&compiled, radix, 100, &mut leaves);
        let expected: BTreeMap<_, _> = payout
            .participants
            .iter()
            .map(|share| {
                (
                    Address::p2tr(
                        &Secp256k1::verification_only(),
                        share.key,
                        None,
                        Network::Regtest,
                    )
                    .script_pubkey(),
                    share.amount.to_sat(),
                )
            })
            .collect();
        assert_eq!(leaves, expected);
        assert_eq!(
            transactions,
            transaction_count(miners.into(), radix).unwrap()
        );
        assert_eq!(leaves.values().sum::<u64>() + transactions * 100, 50_003);
        let amounts: Vec<_> = leaves.values().copied().collect();
        assert!(amounts.iter().max().unwrap() - amounts.iter().min().unwrap() <= 1);
    }
}

#[test]
fn remainder_assignment_is_independent_of_key_order() {
    let mut reversed = keys(5);
    reversed.reverse();
    let a = MiningPayout::from_reward(keys(5), Amount::from_sat(10_003), 4, Amount::from_sat(100))
        .unwrap();
    let b = MiningPayout::from_reward(reversed, Amount::from_sat(10_003), 4, Amount::from_sat(100))
        .unwrap();
    assert_eq!(
        serde_json::to_value(&a).unwrap(),
        serde_json::to_value(&b).unwrap()
    );
    let json = serde_json::to_value(&a).unwrap();
    assert!(json["participants"][0]["amount"].is_u64());
    let decoded: MiningPayout = serde_json::from_value(json).unwrap();
    assert_eq!(
        decoded.funding_required().unwrap(),
        a.funding_required().unwrap()
    );
}

#[test]
fn malformed_or_underfunded_trees_return_errors() {
    for radix in [0, 1] {
        assert!(MiningPayout::from_reward(
            keys(3),
            Amount::from_sat(1000),
            radix,
            Amount::from_sat(100)
        )
        .is_err());
        let mut payout =
            MiningPayout::from_reward(keys(3), Amount::from_sat(1000), 2, Amount::from_sat(100))
                .unwrap();
        payout.radix = radix;
        assert!(payout.compile(context(1000)).is_err());
    }
    assert!(MiningPayout::from_reward(vec![], Amount::from_sat(1000), 4, Amount::ZERO).is_err());
    assert!(MiningPayout::from_reward(
        vec![keys(1)[0]; 2],
        Amount::from_sat(1000),
        4,
        Amount::ZERO
    )
    .is_err());
    assert!(
        MiningPayout::from_reward(keys(5), Amount::from_sat(204), 4, Amount::from_sat(100))
            .is_err()
    );
    let payout =
        MiningPayout::from_reward(keys(5), Amount::from_sat(205), 4, Amount::from_sat(100))
            .unwrap();
    assert!(matches!(
        payout.compile(context(204)),
        Err(CompilationError::OutOfFunds)
    ));
    assert!(MiningPayout::from_reward(
        keys(3),
        Amount::from_sat(u64::MAX),
        2,
        Amount::from_sat(u64::MAX)
    )
    .is_err());
    let payout = MiningPayout {
        participants: keys(2)
            .into_iter()
            .map(|key| PoolShare {
                key,
                amount: Amount::from_sat(u64::MAX),
            })
            .collect(),
        radix: 2,
        fee_sats_per_tx: Amount::ZERO,
    };
    assert!(payout.funding_required().is_err());
}
