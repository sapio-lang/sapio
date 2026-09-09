use bitcoin::secp256k1::{Secp256k1, SecretKey};
use bitcoin::{Amount, Network};
use sapio::contract::{CompilationError, Context, Contract};
use sapio::{declare, guard};
use sapio_base::effects::EffectPath;
use sapio_base::plugin_args::{Ordinal, OrdinalsInfo};
use sapio_base::Clause;
use sapio_ctv_emulator_trait::CTVAvailable;
use std::cell::RefCell;
use std::sync::Arc;

fn context(ranges: &[(u64, u64)]) -> Context {
    Context::new(
        Network::Regtest,
        Amount::from_sat(10),
        Arc::new(CTVAvailable),
        EffectPath::try_from("ordinals").unwrap(),
        Arc::new(Default::default()),
        Some(OrdinalsInfo(
            ranges
                .iter()
                .map(|(start, end)| (Ordinal(*start), Ordinal(*end)))
                .collect(),
        )),
    )
}

fn sats(context: &Context) -> Vec<u64> {
    context
        .get_ordinals()
        .as_ref()
        .unwrap()
        .0
        .iter()
        .flat_map(|(start, end)| start.0..end.0)
        .collect()
}

#[test]
fn splits_ordinal_ranges_without_losing_or_duplicating_satoshis() {
    let ranges = [(100, 105), (200, 205)];
    let original = sats(&context(&ranges));
    for amount in [0, 3, 5, 7, 10] {
        let allocated = context(&ranges)
            .with_amount(Amount::from_sat(amount))
            .unwrap();
        let remaining = context(&ranges)
            .spend_amount(Amount::from_sat(amount))
            .unwrap();
        assert_eq!(sats(&allocated), original[..amount as usize]);
        assert_eq!(sats(&remaining), original[amount as usize..]);
        assert_eq!(allocated.funds().as_sat(), amount);
        assert_eq!(remaining.funds().as_sat(), 10 - amount);
    }
}

#[test]
fn rejects_reversed_or_insufficient_ranges() {
    for ranges in [&[(105, 100)][..], &[(100, 102)][..]] {
        assert!(matches!(
            context(ranges).with_amount(Amount::from_sat(3)),
            Err(CompilationError::OrdinalsError(_))
        ));
        assert!(matches!(
            context(ranges).spend_amount(Amount::from_sat(3)),
            Err(CompilationError::OrdinalsError(_))
        ));
    }
}

#[test]
fn external_funds_follow_all_tracked_input_sats() {
    let ranges = [(100, 105), (200, 205)];
    assert!(context(&ranges).add_amount(Amount::ONE_SAT).is_err());
    assert!(context(&ranges)
        .spend_amount(Amount::from_sat(9))
        .unwrap()
        .add_amount(Amount::ONE_SAT)
        .is_err());
    let exhausted = context(&ranges).spend_amount(Amount::from_sat(10)).unwrap();
    assert_eq!(sats(&exhausted), Vec::<u64>::new());
    let external = exhausted.add_amount(Amount::from_sat(7)).unwrap();
    assert!(external.get_ordinals().is_none());
    assert_eq!(external.funds().as_sat(), 7);
    assert_eq!(
        external.spend_amount(Amount::from_sat(7)).unwrap().funds(),
        Amount::ZERO
    );
    // A malformed context with untracked existing money is not exhausted.
    assert!(context(&[]).add_amount(Amount::ONE_SAT).is_err());
}

#[test]
fn adding_external_funds_rejects_overflow() {
    let ctx = Context::new(
        Network::Regtest,
        Amount::from_sat(u64::MAX),
        Arc::new(CTVAvailable),
        EffectPath::try_from("overflow").unwrap(),
        Arc::new(Default::default()),
        None,
    );
    assert!(ctx.add_amount(Amount::ONE_SAT).is_err());
}

#[derive(Default)]
struct ObserveOrdinals(RefCell<Vec<(Amount, Option<OrdinalsInfo>)>>);

impl ObserveOrdinals {
    #[guard]
    fn spend(self, ctx: Context) {
        self.0
            .borrow_mut()
            .push((ctx.funds(), ctx.get_ordinals().clone()));
        Clause::Key(
            SecretKey::from_slice(&[1; 32])
                .unwrap()
                .keypair(&Secp256k1::new())
                .x_only_public_key()
                .0,
        )
    }
}

impl Contract for ObserveOrdinals {
    declare! {finish, Self::spend}
    declare! {non updatable}
}

#[test]
fn builder_outputs_receive_input_order_ordinals_and_fees_consume_only_the_tail() {
    let recipient = ObserveOrdinals::default();
    // The lower ordinal numbers belong to the later input range. Allocation
    // must follow input order rather than sort by ordinal number.
    let template: sapio::template::Template = context(&[(200, 205), (100, 105)])
        .template()
        .add_output(Amount::from_sat(6), &recipient, None)
        .unwrap()
        .add_output(Amount::from_sat(2), &recipient, None)
        .unwrap()
        .add_fees(Amount::ONE_SAT)
        .unwrap()
        .add_fees(Amount::ONE_SAT)
        .unwrap()
        .into();
    assert_eq!(
        *recipient.0.borrow(),
        vec![
            (
                Amount::from_sat(6),
                Some(OrdinalsInfo(vec![
                    (Ordinal(200), Ordinal(205)),
                    (Ordinal(100), Ordinal(101)),
                ])),
            ),
            (
                Amount::from_sat(2),
                Some(OrdinalsInfo(vec![(Ordinal(101), Ordinal(103))])),
            ),
        ]
    );
    assert_eq!(
        template
            .tx
            .output
            .iter()
            .map(|out| out.value)
            .collect::<Vec<_>>(),
        [6, 2]
    );
    assert_eq!(template.max - template.total_amount(), Amount::from_sat(2));
    assert_eq!(template.required_input_amount, Amount::from_sat(10));

    assert!(matches!(
        context(&[(200, 205), (100, 105)])
            .template()
            .add_output(Amount::from_sat(8), &recipient, None)
            .unwrap()
            .add_fees(Amount::ONE_SAT)
            .unwrap()
            .add_fees(Amount::from_sat(2)),
        Err(CompilationError::OutOfFunds)
    ));
}
