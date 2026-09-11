//! Compile a small payment contract without a node, signer, or WASM runtime.
//! Native CTV is a research target; this example does not fund or broadcast it.
use bitcoin::{Address, Amount, Network};
use sapio::contract::{Compilable, Compiled, Context, Contract};
use sapio::{declare, then};
use sapio_base::covenant::LoweringPlan;
use sapio_base::effects::EffectPath;
use std::sync::Arc;

struct Payment {
    destination: Compiled,
}

impl Payment {
    #[then]
    fn pay(self, ctx: Context) {
        ctx.template()
            .add_output(Amount::from_sat(1_000), &self.destination, None)?
            .set_min_feerate(Amount::from_sat(1))
            .add_fees(Amount::from_sat(500))?
            .into()
    }
}

impl Contract for Payment {
    declare! {then, Self::pay}
    declare! {non updatable}
}

fn compile_payment(funding: u64) -> Result<Compiled, Box<dyn std::error::Error>> {
    let destination = "bcrt1qumrrqgt7e3a7damzm8x97m6sjs20u8hjw2hcjj"
        .parse::<Address<bitcoin::address::NetworkUnchecked>>()?
        .require_network(Network::Regtest)?;
    let contract = Payment {
        destination: Compiled::from_address(destination, bitcoin::Amount::ZERO),
    };
    let compiled = contract.compile(Context::new(
        Network::Regtest,
        Amount::from_sat(funding),
        LoweringPlan::Native,
        EffectPath::try_from("payment")?,
        Arc::new(Default::default()),
        None,
    ))?;
    compiled.validate()?;
    Ok(compiled)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let compiled = compile_payment(1_500)?;
    serde_json::to_writer_pretty(
        std::io::stdout().lock(),
        &serde_json::json!({
            "network": "regtest",
            "enforcement": "native_ctv_research",
            "funding_satoshis": 1_500,
            "contract": compiled,
        }),
    )?;
    Ok(())
}

#[test]
fn payment_preserves_destination_value_and_fee_reserve() {
    let compiled = compile_payment(1_500).unwrap();
    assert_eq!(compiled.ctv_to_tx.len(), 1);
    let template = compiled.ctv_to_tx.values().next().unwrap();
    assert_eq!(template.tx.input.len(), 1);
    assert_eq!(template.tx.output.len(), 1);
    assert_eq!(template.tx.output[0].value.to_sat(), 1_000);
    let destination = "bcrt1qumrrqgt7e3a7damzm8x97m6sjs20u8hjw2hcjj"
        .parse::<Address<bitcoin::address::NetworkUnchecked>>()
        .unwrap()
        .require_network(Network::Regtest)
        .unwrap();
    assert_eq!(
        template.tx.output[0].script_pubkey,
        destination.script_pubkey()
    );
    assert_eq!(template.required_input_amount.to_sat(), 1_500);
    assert!(compile_payment(1_499).is_err());
}
