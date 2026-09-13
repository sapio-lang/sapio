use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
use bitcoin::{Address, Amount, FeeRate, Network};
use sapio::contract::object::ObjectMetadata;
use sapio::contract::{CompilationError, Compiled, Context, Contract};
use sapio::ordinals::{Ordinal, OrdinalsInfo};
use sapio::template::{OutputAmount, OutputMeta, PlanError, Surplus};
use sapio::{declare, guard};
use sapio_base::covenant::LoweringPlan;
use sapio_base::timelocks::{AbsHeight, AbsTime, RelHeight, RelTime};
use sapio_base::{CTVHash, Clause};
use std::sync::{Arc, Mutex};

fn context(funds: u64, ordinals: Option<OrdinalsInfo>) -> Context {
    Context::new(
        Network::Regtest,
        Amount::from_sat(funds),
        LoweringPlan::Native,
        "plan".try_into().unwrap(),
        Arc::new(Default::default()),
        ordinals,
    )
}

fn key() -> bitcoin::XOnlyPublicKey {
    Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[9; 32]).unwrap())
        .x_only_public_key()
        .0
}

fn destination(minimum: u64) -> Compiled {
    Compiled::from_address(
        Address::p2tr(&Secp256k1::new(), key(), None, Network::Regtest),
        Amount::from_sat(minimum),
    )
}

#[test]
fn remainder_resolves_before_exact_outputs_without_reordering() {
    let destination = destination(0);
    let mut plan = context(1_000, None).template_plan();
    let change = plan
        .output("change", OutputAmount::Remainder, &destination)
        .unwrap();
    let payout = plan
        .output(
            "recipient",
            OutputAmount::Exact(Amount::from_sat(300)),
            &destination,
        )
        .unwrap();
    assert_eq!(change.name(), "change");
    assert_eq!(payout.name(), "recipient");
    plan.output_metadata(
        &payout,
        OutputMeta::from([("memo", serde_json::json!("payment"))]),
    )
    .unwrap();
    plan.reserve_fees(Amount::from_sat(100));
    plan.reserve_fees(Amount::from_sat(50));
    let template = plan.finish().unwrap();
    assert_eq!(
        template
            .tx
            .output
            .iter()
            .map(|o| o.value.to_sat())
            .collect::<Vec<_>>(),
        [600, 300]
    );
    assert_eq!(template.outputs[1].added_metadata.extra["memo"], "payment");
    assert_eq!(template.ctv, template.tx.get_ctv_hash(0));
    assert_eq!(template.max, Amount::from_sat(1_000));
    assert_eq!(template.required_input_amount, Amount::from_sat(1_000));
    let funding = template.funding_constraints.as_ref().unwrap();
    assert_eq!(funding.inputs[0].name, "contract");
    assert_eq!(funding.inputs[0].minimum.to_sat(), 1_000);
    assert_eq!(funding.outputs, ["change", "recipient"]);
    assert_eq!(funding.maximum_fee.to_sat(), 100);
    let restored: sapio::template::Template =
        serde_json::from_value(serde_json::to_value(&template).unwrap()).unwrap();
    assert_eq!(restored, template);
}

#[test]
fn sponsor_requirements_keep_input_order_and_residual_feerate() {
    let destination = destination(0);
    let mut plan = context(700, None).template_plan();
    let sponsor = plan.input("sponsor", Amount::from_sat(300)).unwrap();
    let helper = plan.input("later-sponsor", Amount::ZERO).unwrap();
    plan.require_older(&sponsor, RelHeight::try_from(7).unwrap().into())
        .unwrap();
    plan.require_older(&sponsor, RelHeight::try_from(5).unwrap().into())
        .unwrap();
    plan.require_older(&helper, RelHeight::try_from(2).unwrap().into())
        .unwrap();
    plan.output(
        "recipient",
        OutputAmount::Exact(Amount::from_sat(900)),
        &destination,
    )
    .unwrap();
    plan.reserve_fees(Amount::from_sat(100));
    plan.surplus(Surplus::Fees {
        maximum: Amount::from_sat(400),
    });
    plan.require_feerate(FeeRate::from_sat_per_kwu(375));
    plan.require_feerate(FeeRate::from_sat_per_kwu(250));
    let template = plan.finish().unwrap();
    assert_eq!(template.tx.input[1].sequence.to_consensus_u32(), 7);
    assert_eq!(template.tx.input[2].sequence.to_consensus_u32(), 2);
    assert_eq!(template.required_input_amount.to_sat(), 700);
    assert!(template.min_feerate_sats_vbyte.is_none());
    let funding = template.funding_constraints.unwrap();
    assert_eq!(
        funding
            .inputs
            .iter()
            .map(|input| (input.name.as_str(), input.minimum.to_sat()))
            .collect::<Vec<_>>(),
        [("contract", 700), ("sponsor", 300), ("later-sponsor", 0)]
    );
    assert_eq!(funding.maximum_fee.to_sat(), 400);
    assert_eq!(
        funding.minimum_feerate,
        Some(FeeRate::from_sat_per_kwu(375))
    );
}

#[test]
fn lock_conflicts_name_the_input_and_preserve_previous_requirements() {
    let destination = destination(0);
    let mut plan = context(1, None).template_plan();
    let input = plan.contract_input();
    plan.require_older(&input, RelHeight::try_from(10).unwrap().into())
        .unwrap();
    assert!(
        matches!(plan.require_older(&input, RelTime::try_from(512).unwrap().into()), Err(PlanError::ConflictingRelativeLocks { input, .. }) if input == "contract")
    );
    plan.require_after(AbsHeight::try_from(500).unwrap().into())
        .unwrap();
    assert!(matches!(
        plan.require_after(AbsTime::try_from(500_000_001).unwrap().into()),
        Err(PlanError::ConflictingAbsoluteLocks { .. })
    ));
    plan.require_after(AbsHeight::try_from(300).unwrap().into())
        .unwrap();
    plan.output("recipient", OutputAmount::Remainder, &destination)
        .unwrap();
    let template = plan.finish().unwrap();
    assert_eq!(template.tx.lock_time.to_consensus_u32(), 500);
    assert_eq!(template.tx.input[0].sequence.to_consensus_u32(), 10);
}

#[test]
fn ambiguous_and_missing_allocations_report_the_actual_conflict() {
    let destination = destination(0);
    let mut plan = context(1_000, None).template_plan();
    plan.output("first", OutputAmount::Remainder, &destination)
        .unwrap();
    assert!(
        matches!(plan.output("second", OutputAmount::Remainder, &destination), Err(PlanError::MultipleRemainders { first, second }) if first == "first" && second == "second")
    );
    assert!(matches!(
        plan.output("first", OutputAmount::Exact(Amount::ZERO), &destination),
        Err(PlanError::DuplicateName { kind: "output", .. })
    ));
    assert!(matches!(
        plan.input("contract", Amount::ZERO),
        Err(PlanError::DuplicateName { kind: "input", .. })
    ));
    assert!(matches!(
        plan.input("", Amount::ZERO),
        Err(PlanError::EmptyName)
    ));
    assert_eq!(plan.finish().unwrap().tx.output[0].value.to_sat(), 1_000);

    let mut unallocated = context(1_000, None).template_plan();
    unallocated
        .output(
            "payment",
            OutputAmount::Exact(Amount::from_sat(900)),
            &destination,
        )
        .unwrap();
    assert!(
        matches!(unallocated.finish(), Err(PlanError::UnallocatedFunds { amount }) if amount.to_sat() == 100)
    );

    let mut underfunded = context(1_000, None).template_plan();
    underfunded
        .output(
            "payment",
            OutputAmount::Exact(Amount::from_sat(950)),
            &destination,
        )
        .unwrap();
    underfunded.reserve_fees(Amount::from_sat(100));
    assert!(
        matches!(underfunded.finish(), Err(PlanError::InsufficientFunds { available, outputs, fees }) if (available.to_sat(), outputs.to_sat(), fees.to_sat()) == (1_000, 950, 100))
    );
}

#[test]
fn surplus_fees_require_an_explicit_sufficient_ceiling() {
    let destination = destination(0);
    for (maximum, accepted) in [(149, false), (150, true), (200, true)] {
        let mut plan = context(1_000, None).template_plan();
        plan.output(
            "payment",
            OutputAmount::Exact(Amount::from_sat(850)),
            &destination,
        )
        .unwrap();
        plan.reserve_fees(Amount::from_sat(100));
        plan.surplus(Surplus::Fees {
            maximum: Amount::from_sat(maximum),
        });
        match plan.finish() {
            Ok(template) => {
                assert!(accepted);
                assert_eq!(
                    template.max.to_sat() - template.total_amount().to_sat(),
                    150
                );
                assert_eq!(
                    template.funding_constraints.unwrap().maximum_fee.to_sat(),
                    maximum
                );
            }
            Err(PlanError::FeeLimit { fee, maximum: cap }) => {
                assert!(!accepted);
                assert_eq!((fee.to_sat(), cap.to_sat()), (150, maximum));
            }
            Err(error) => panic!("unexpected {error}"),
        }
    }
    let mut plan = context(100, None).template_plan();
    plan.output("change", OutputAmount::Remainder, &destination)
        .unwrap();
    plan.reserve_fees(Amount::from_sat(100));
    plan.surplus(Surplus::Fees {
        maximum: Amount::from_sat(99),
    });
    assert!(matches!(plan.finish(), Err(PlanError::FeeLimit { .. })));
}

#[test]
fn overflow_and_child_funding_errors_identify_the_declaration() {
    let mut overflow = context(u64::MAX, None).template_plan();
    overflow.input("sponsor", Amount::ONE_SAT).unwrap();
    assert!(
        matches!(overflow.finish(), Err(PlanError::AmountOverflow { at }) if at.contains("sponsor"))
    );

    let child = destination(1_000);
    let mut plan = context(900, None).template_plan();
    plan.output("child", OutputAmount::Remainder, &child)
        .unwrap();
    assert!(
        matches!(plan.finish(), Err(PlanError::Compilation { at, source }) if at == "output \"child\"" && matches!(*source, CompilationError::UnderfundedOutput { available, required, .. } if available.to_sat() == 900 && required.to_sat() == 1_000))
    );
}

type Observations = Arc<Mutex<Vec<(u64, Option<OrdinalsInfo>)>>>;

struct Probe(Observations);

impl Probe {
    #[guard]
    fn spend(self, _ctx: Context) {
        Clause::Key(key())
    }
}

impl Contract for Probe {
    declare! {finish, Self::spend}

    fn metadata(&self, ctx: Context) -> Result<ObjectMetadata, CompilationError> {
        self.0
            .lock()
            .unwrap()
            .push((ctx.funds().to_sat(), ctx.get_ordinals().clone()));
        Ok(ObjectMetadata::default())
    }
}

#[test]
fn children_compile_once_in_declared_ordinal_order() {
    let observed = Observations::default();
    let probe = Probe(observed.clone());
    let ordinals = OrdinalsInfo(vec![
        (Ordinal(100), Ordinal(105)),
        (Ordinal(10), Ordinal(15)),
    ]);
    let mut plan = context(10, Some(ordinals)).template_plan();
    plan.output("first", OutputAmount::Remainder, &probe)
        .unwrap();
    plan.output("second", OutputAmount::Exact(Amount::from_sat(5)), &probe)
        .unwrap();
    plan.reserve_fees(Amount::ONE_SAT);
    let template = plan.finish().unwrap();
    assert_eq!(
        template.funding_constraints.unwrap().outputs,
        ["first", "second"]
    );
    assert_eq!(
        *observed.lock().unwrap(),
        [
            (4, Some(OrdinalsInfo(vec![(Ordinal(100), Ordinal(104))]))),
            (
                5,
                Some(OrdinalsInfo(vec![
                    (Ordinal(104), Ordinal(105)),
                    (Ordinal(10), Ordinal(14))
                ]))
            ),
        ]
    );
}

#[test]
fn unknown_sponsor_sats_follow_the_tracked_prefix_or_fail_explicitly() {
    let observed = Observations::default();
    let probe = Probe(observed.clone());
    let ranges = OrdinalsInfo(vec![(Ordinal(50), Ordinal(60))]);
    let mut plan = context(10, Some(ranges.clone())).template_plan();
    plan.input("sponsor", Amount::from_sat(5)).unwrap();
    plan.output("tracked", OutputAmount::Exact(Amount::from_sat(10)), &probe)
        .unwrap();
    plan.output("untracked", OutputAmount::Remainder, &probe)
        .unwrap();
    plan.reserve_fees(Amount::ONE_SAT);
    assert_eq!(
        plan.finish()
            .unwrap()
            .tx
            .output
            .iter()
            .map(|o| o.value.to_sat())
            .collect::<Vec<_>>(),
        [10, 4]
    );
    assert_eq!(
        *observed.lock().unwrap(),
        [(10, Some(ranges.clone())), (4, None)]
    );

    let mut crossing = context(10, Some(ranges)).template_plan();
    crossing.input("sponsor", Amount::from_sat(5)).unwrap();
    crossing
        .output("mixed", OutputAmount::Remainder, &probe)
        .unwrap();
    assert!(
        matches!(crossing.finish(), Err(PlanError::UnknownOrdinalAllocation { at, tracked_remaining }) if at == "output \"mixed\"" && tracked_remaining.to_sat() == 10)
    );
    assert_eq!(observed.lock().unwrap().len(), 2);
}

#[test]
fn ordinal_ranges_must_cover_the_budget_without_overlap() {
    let observed = Observations::default();
    let probe = Probe(observed.clone());
    let mut short =
        context(10, Some(OrdinalsInfo(vec![(Ordinal(50), Ordinal(59))]))).template_plan();
    short
        .output("tracked", OutputAmount::Remainder, &probe)
        .unwrap();
    assert!(
        matches!(short.finish(), Err(PlanError::OrdinalFunding { available, tracked }) if available.to_sat() == 10 && tracked.to_sat() == 9)
    );
    let mut overlap = context(
        10,
        Some(OrdinalsInfo(vec![
            (Ordinal(50), Ordinal(55)),
            (Ordinal(54), Ordinal(59)),
        ])),
    )
    .template_plan();
    overlap
        .output("tracked", OutputAmount::Remainder, &probe)
        .unwrap();
    assert!(
        matches!(overlap.finish(), Err(PlanError::Compilation { at, .. }) if at == "contract input ordinals")
    );
    assert!(observed.lock().unwrap().is_empty());
}

#[test]
fn explicit_ordinal_placement_checks_offsets_without_moving_outputs() {
    let destination = destination(0);
    let ranges = OrdinalsInfo(vec![
        (Ordinal(u64::MAX - 5), Ordinal(u64::MAX)),
        (Ordinal(10), Ordinal(15)),
    ]);
    for (ordinal, offset, expected) in [
        (Ordinal(10), 1, "ok"),
        (Ordinal(10), 0, "offset"),
        (Ordinal(u64::MAX - 5), 0, "outside"),
        (Ordinal(50), 0, "unknown"),
        (Ordinal(10), 6, "bounds"),
    ] {
        let mut plan = context(10, Some(ranges.clone())).template_plan();
        plan.output(
            "first",
            OutputAmount::Exact(Amount::from_sat(4)),
            &destination,
        )
        .unwrap();
        let second = plan
            .output("second", OutputAmount::Remainder, &destination)
            .unwrap();
        plan.require_ordinal(&second, ordinal, offset).unwrap();
        match (expected, plan.finish()) {
            ("ok", Ok(template)) => assert_eq!(
                template
                    .tx
                    .output
                    .iter()
                    .map(|output| output.value.to_sat())
                    .collect::<Vec<_>>(),
                [4, 6]
            ),
            (
                "offset",
                Err(PlanError::OrdinalPlacement {
                    output,
                    required: 0,
                    actual: Some(1),
                    ..
                }),
            ) => assert_eq!(output, "second"),
            ("outside", Err(PlanError::OrdinalPlacement { actual: None, .. })) => {}
            ("unknown", Err(PlanError::UnknownOrdinal { .. })) => {}
            ("bounds", Err(PlanError::InvalidOrdinalOffset { .. })) => {}
            (_, result) => panic!("unexpected {expected}: {result:?}"),
        }
    }
    let mut unknown = context(10, None).template_plan();
    let output = unknown
        .output("unknown", OutputAmount::Remainder, &destination)
        .unwrap();
    unknown.require_ordinal(&output, Ordinal(10), 0).unwrap();
    assert!(matches!(
        unknown.finish(),
        Err(PlanError::UnknownOrdinal { .. })
    ));
}
