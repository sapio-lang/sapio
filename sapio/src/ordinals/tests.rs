use super::*;
use crate::contract::Compiled;
use bitcoin::{Address, Network};
use sapio_base::effects::EffectPath;
use sapio_ctv_emulator_trait::CTVAvailable;
use std::sync::Arc;

fn info(ranges: &[(u64, u64)]) -> OrdinalsInfo {
    OrdinalsInfo(
        ranges
            .iter()
            .map(|(a, b)| (Ordinal(*a), Ordinal(*b)))
            .collect(),
    )
}
fn spec(ordinals: &[u64], payouts: &[u64], payins: &[u64], fees: u64) -> OrdinalSpec {
    OrdinalSpec {
        ordinals: ordinals.iter().copied().map(Ordinal).collect(),
        payouts: payouts.iter().copied().map(Amount::from_sat).collect(),
        payins: payins.iter().copied().map(Amount::from_sat).collect(),
        fees: Amount::from_sat(fees),
    }
}
fn context(ranges: OrdinalsInfo) -> Context {
    Context::new(
        Network::Regtest,
        ranges.total().unwrap(),
        Arc::new(CTVAvailable),
        EffectPath::try_from("plan").unwrap(),
        Arc::new(Default::default()),
        Some(ranges),
    )
}
fn destination() -> Compiled {
    Compiled::from_address(
        "bcrt1qumrrqgt7e3a7damzm8x97m6sjs20u8hjw2hcjj"
            .parse::<Address>()
            .unwrap(),
        None,
    )
}

#[test]
fn ordinal_outputs_follow_input_order_and_preserve_complete_accounting() {
    let ranges = info(&[(1000, 2000), (0, 1000)]);
    let request = spec(&[10, 1010], &[7, 7, 300], &[], 10);
    let plan = ranges.output_plan(&request).unwrap();
    let wanted: Vec<_> = plan
        .0
        .iter()
        .filter_map(|step| match step {
            PlanStep::Ordinal(ordinal) => Some(*ordinal),
            _ => None,
        })
        .collect();
    assert_eq!(wanted, [Ordinal(1010), Ordinal(10)]);
    let target = destination();
    let mut payouts = BTreeMap::new();
    for amount in &request.payouts {
        payouts
            .entry(*amount)
            .or_insert_with(Vec::new)
            .push((&target as &dyn Compilable, None));
    }
    let ordinals = request
        .ordinals
        .iter()
        .map(|ordinal| (*ordinal, (&target as &dyn Compilable, None)))
        .collect();
    let template: crate::template::Template = plan
        .build_plan(context(ranges.clone()), payouts, ordinals, (&target, None))
        .unwrap()
        .into();
    let input_sats: Vec<_> = ranges.0.iter().flat_map(|(a, b)| a.0..b.0).collect();
    let mut offset = 0usize;
    let mut observed = Vec::new();
    for output in &template.tx.output {
        if output.value == 501 {
            observed.push(input_sats[offset]);
        }
        offset += output.value as usize;
    }
    assert_eq!(observed, [1010, 10]);
    assert_eq!(offset, 1990);
    assert_eq!(template.max.as_sat(), 2000);
}

#[test]
fn auxiliary_inputs_pay_the_seller_change_and_fee_after_known_sats() {
    let ranges = info(&[(1000, 2000)]);
    let request = spec(&[1010], &[800], &[1000], 100);
    let plan = ranges.output_plan(&request).unwrap();
    assert_eq!(
        plan.0,
        [
            PlanStep::Change(Amount::from_sat(10)),
            PlanStep::Ordinal(Ordinal(1010)),
            PlanStep::Change(Amount::from_sat(489)),
            PlanStep::PayIn(Amount::from_sat(1000)),
            PlanStep::Payout(Amount::from_sat(800)),
            PlanStep::Change(Amount::from_sat(100)),
            PlanStep::Fee(Amount::from_sat(100)),
        ]
    );
    let target = destination();
    let payouts = BTreeMap::from([(
        Amount::from_sat(800),
        vec![(&target as &dyn Compilable, None)],
    )]);
    let ordinals = BTreeMap::from([(Ordinal(1010), (&target as &dyn Compilable, None))]);
    let template: crate::template::Template = plan
        .build_plan(context(ranges), payouts, ordinals, (&target, None))
        .unwrap()
        .into();
    assert_eq!(template.tx.input.len(), 2);
    assert_eq!(
        template
            .tx
            .output
            .iter()
            .map(|out| out.value)
            .collect::<Vec<_>>(),
        [10, 501, 489, 800, 100]
    );
    assert_eq!(template.max.as_sat(), 2000);
}

#[test]
fn gaps_smaller_than_any_payout_become_change_without_underflow() {
    let plan = info(&[(0, 1000)])
        .output_plan(&spec(&[1], &[200], &[], 10))
        .unwrap();
    assert_eq!(plan.0.first(), Some(&PlanStep::Change(Amount::ONE_SAT)));
    let near_max = info(&[(0, 1000), (u64::MAX - 1000, u64::MAX)]);
    near_max
        .output_plan(&spec(&[u64::MAX - 999], &[], &[], 10))
        .unwrap();
}

#[test]
fn malformed_missing_overlapping_and_underfunded_requests_are_errors() {
    for ranges in [info(&[(10, 1)]), info(&[(1, 1)]), info(&[(0, 10), (9, 20)])] {
        assert!(ranges.output_plan(&spec(&[], &[], &[], 0)).is_err());
    }
    let ranges = info(&[(0, 1000)]);
    for request in [
        spec(&[1000], &[], &[], 0),
        spec(&[999], &[], &[], 0),
        spec(&[0, 1], &[], &[], 0),
        spec(&[0], &[500], &[], 0),
        spec(&[], &[u64::MAX, 1], &[], 0),
        spec(&[], &[], &[u64::MAX, 1], 0),
        spec(&[], &[0], &[], 0),
        spec(&[], &[], &[0], 0),
        spec(&[499], &[], &[], 1),
    ] {
        assert!(ranges.output_plan(&request).is_err());
    }
}

#[test]
fn executing_a_plan_rechecks_actual_ordinal_positions_and_destinations() {
    let ranges = info(&[(1000, 2000)]);
    let target = destination();
    let request = spec(&[1010], &[], &[], 10);
    let ordinals = || BTreeMap::from([(Ordinal(1010), (&target as &dyn Compilable, None))]);
    assert!(ranges
        .output_plan(&request)
        .unwrap()
        .build_plan(
            context(info(&[(0, 1000)])),
            BTreeMap::new(),
            ordinals(),
            (&target, None)
        )
        .is_err());
    assert!(ranges
        .output_plan(&request)
        .unwrap()
        .build_plan(
            context(ranges.clone()),
            BTreeMap::new(),
            BTreeMap::new(),
            (&target, None)
        )
        .is_err());
}

#[test]
fn executing_a_plan_rejects_malformed_actual_ranges_and_amounts() {
    let ranges = info(&[(0, 1000)]);
    let request = spec(&[0], &[], &[], 0);
    let target = destination();
    for actual in [info(&[(0, 500), (0, 500)]), info(&[(0, 999)])] {
        let ctx = Context::new(
            Network::Regtest,
            Amount::from_sat(1000),
            Arc::new(CTVAvailable),
            EffectPath::try_from("plan").unwrap(),
            Arc::new(Default::default()),
            Some(actual),
        );
        assert!(ranges
            .output_plan(&request)
            .unwrap()
            .build_plan(
                ctx,
                BTreeMap::new(),
                BTreeMap::from([(Ordinal(0), (&target as &dyn Compilable, None))]),
                (&target, None),
            )
            .is_err());
    }
}
