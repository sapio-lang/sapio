use bitcoin::{Address, Amount, Network};
use sapio::contract::{Compilable, Compiled, Context, Contract};
use sapio::{declare, then};
use sapio_base::covenant::LoweringPlan;
use sapio_base::effects::EffectPath;
use std::str::FromStr;
use std::sync::Arc;

struct RepeatedPayment;

impl RepeatedPayment {
    #[then]
    fn pay(self, ctx: Context) {
        let destination = Compiled::from_address(
            Address::from_str("bcrt1qumrrqgt7e3a7damzm8x97m6sjs20u8hjw2hcjj").unwrap(),
            bitcoin::Amount::ZERO,
        );
        ctx.template()
            .add_output(Amount::from_sat(1_000), &destination, None)?
            .into()
    }
}

impl Contract for RepeatedPayment {
    declare! {then, Self::pay, Self::pay, Self::pay}
    declare! {non updatable}
}

#[test]
fn compiles_three_actions_with_the_same_name() {
    let compiled = RepeatedPayment
        .compile(Context::new(
            Network::Regtest,
            Amount::from_sat(1_000),
            LoweringPlan::Native,
            EffectPath::try_from("repeated_payment").unwrap(),
            Arc::new(Default::default()),
            None,
        ))
        .unwrap();
    assert_eq!(compiled.ctv_to_tx.len(), 1);
}
