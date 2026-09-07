//! Compile a small payment contract without a node, signer, or WASM runtime.
//! Native CTV is a research target; this example does not fund or broadcast it.
use bitcoin::{Address, Amount, Network};
use sapio::contract::{Compilable, Compiled, Context, Contract};
use sapio::{declare, then};
use sapio_base::effects::EffectPath;
use sapio_ctv_emulator_trait::CTVAvailable;
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let destination: Address = "bcrt1qumrrqgt7e3a7damzm8x97m6sjs20u8hjw2hcjj".parse()?;
    let contract = Payment {
        destination: Compiled::from_address(destination, None),
    };
    let compiled = contract.compile(Context::new(
        Network::Regtest,
        Amount::from_sat(1_500),
        Arc::new(CTVAvailable),
        EffectPath::try_from("payment")?,
        Arc::new(Default::default()),
        None,
    ))?;
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
