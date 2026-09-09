use bitcoin::{Address, Amount, Network};
use sapio::contract::{Compilable, CompilationError, Compiled, Context, Contract};
use sapio::{declare, then};
use sapio_base::effects::EffectPath;
use sapio_ctv_emulator_trait::CTVAvailable;
use std::str::FromStr;
use std::sync::Arc;

struct Payment {
    fee: u64,
    rates: Vec<u64>,
    extra_input: bool,
}

impl Payment {
    #[then]
    fn pay(self, ctx: Context) {
        let destination = Compiled::from_address(
            Address::from_str("bcrt1qumrrqgt7e3a7damzm8x97m6sjs20u8hjw2hcjj").unwrap(),
            bitcoin::Amount::ZERO,
        );
        let mut builder = ctx
            .template()
            .add_output(Amount::from_sat(1_000), &destination, None)?;
        for rate in &self.rates {
            builder = builder.set_min_feerate(Amount::from_sat(*rate));
        }
        if self.extra_input {
            builder = builder.add_sequence();
        }
        builder.add_fees(Amount::from_sat(self.fee))?.into()
    }
}

impl Contract for Payment {
    declare! {then, Self::pay}
    declare! {non updatable}
}

fn compile(fee: u64, rates: Vec<u64>, extra_input: bool) -> Result<Compiled, CompilationError> {
    Payment {
        fee,
        rates,
        extra_input,
    }
    .compile(Context::new(
        Network::Regtest,
        Amount::from_sat(10_000),
        Arc::new(CTVAvailable),
        EffectPath::try_from("payment").unwrap(),
        Arc::new(Default::default()),
        None,
    ))
}

#[test]
fn accepts_fees_above_the_minimum_in_virtual_bytes() {
    compile(150, vec![1], false).unwrap();
    compile(1_000, vec![1], false).unwrap();
}

#[test]
fn rejects_insufficient_reserved_fees() {
    assert!(matches!(
        compile(0, vec![1], false),
        Err(CompilationError::MinFeerateError)
    ));
}

#[test]
fn repeated_requirements_keep_the_strongest_minimum() {
    for rates in [vec![1, 10], vec![10, 1]] {
        assert!(matches!(
            compile(150, rates, false),
            Err(CompilationError::MinFeerateError)
        ));
    }
}

#[test]
fn rejects_a_feerate_whose_required_amount_overflows() {
    assert!(matches!(
        compile(1_000, vec![u64::MAX], false),
        Err(CompilationError::MinFeerateError)
    ));
}

#[test]
fn rejects_a_minimum_when_additional_input_weights_are_unknown() {
    assert!(matches!(
        compile(1_000, vec![1], true),
        Err(CompilationError::MinFeerateError)
    ));
    compile(1_000, vec![], true).unwrap();
}
