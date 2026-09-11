use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
use bitcoin::{Amount, Network, XOnlyPublicKey};
use sapio::contract::abi::object::SupportedDescriptors;
use sapio::contract::{Compilable, Context, Contract};
use sapio::{declare, guard, then};
use sapio_base::covenant::LoweringPlan;
use sapio_base::effects::EffectPath;
use sapio_base::miniscript::ForEachKey;
use sapio_base::Clause;
use std::collections::BTreeSet;
use std::sync::Arc;

struct TwoGuards {
    first: XOnlyPublicKey,
    second: XOnlyPublicKey,
}

impl TwoGuards {
    #[guard]
    fn first(self, _ctx: Context) {
        Clause::Key(self.first)
    }

    #[guard]
    fn second(self, _ctx: Context) {
        Clause::Key(self.second)
    }

    #[then(guarded_by = "[Self::first, Self::second]")]
    fn pay(self, ctx: Context) {
        ctx.template()
            .add_output(Amount::from_sat(1_000), &self.first, None)?
            .into()
    }
}

impl Contract for TwoGuards {
    declare! {then, Self::pay}
    declare! {non updatable}
}

#[test]
fn compiles_both_guards_with_distinct_metadata_paths() {
    let secp = Secp256k1::new();
    let key = |byte| {
        Keypair::from_secret_key(&secp, &SecretKey::from_slice(&[byte; 32]).unwrap())
            .x_only_public_key()
            .0
    };
    let contract = TwoGuards {
        first: key(1),
        second: key(2),
    };
    let compiled = contract
        .compile(Context::new(
            Network::Regtest,
            Amount::from_sat(1_000),
            LoweringPlan::Native,
            EffectPath::try_from("two_guards").unwrap(),
            Arc::new(Default::default()),
            None,
        ))
        .unwrap();
    let Some(SupportedDescriptors::XOnly(descriptor)) = compiled.descriptor else {
        panic!("expected a Taproot descriptor");
    };
    let mut keys = BTreeSet::new();
    descriptor.for_each_key(|key| {
        keys.insert(*key);
        true
    });
    assert!(keys.contains(&contract.first));
    assert!(keys.contains(&contract.second));
    assert!(compiled.metadata.simps_for_guards.is_empty());
}
