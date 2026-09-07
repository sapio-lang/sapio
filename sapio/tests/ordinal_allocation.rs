use bitcoin::{Amount, Network};
use sapio::contract::{CompilationError, Context};
use sapio_base::effects::EffectPath;
use sapio_base::plugin_args::{Ordinal, OrdinalsInfo};
use sapio_ctv_emulator_trait::CTVAvailable;
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
