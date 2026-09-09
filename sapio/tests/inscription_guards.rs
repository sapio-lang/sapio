use bitcoin::secp256k1::{Secp256k1, SecretKey};
use bitcoin::{Amount, Network, XOnlyPublicKey};
use sapio::contract::actions::ThenFuncAsFinishOrFunc;
use sapio::contract::object::SupportedDescriptors;
use sapio::contract::{Compilable, Context, Contract};
use sapio::miniscript::ord::{envelope::Envelope, Inscription};
use sapio::miniscript::Descriptor;
use sapio::{declare, guard, then};
use sapio_base::covenant::LoweringPlan;
use sapio_base::Clause;
use std::sync::Arc;

fn inscription(body: &[u8]) -> Inscription {
    Inscription::new(Some(b"text/plain".to_vec()), Some(body.to_vec()))
}

fn effect(body: &[u8]) -> Clause {
    Clause::Inscribe(Box::new(inscription(body)), Box::new(Clause::Trivial))
}

fn destination() -> XOnlyPublicKey {
    SecretKey::from_slice(&[1; 32])
        .unwrap()
        .keypair(&Secp256k1::new())
        .x_only_public_key()
        .0
}

fn context() -> Context {
    Context::new(
        Network::Regtest,
        Amount::from_sat(1_000),
        LoweringPlan::Native,
        "inscriptions".try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    )
}

fn encoded_inscriptions(compiled: &sapio::contract::Compiled) -> Vec<Inscription> {
    compiled.validate().unwrap();
    let Some(SupportedDescriptors::XOnly(Descriptor::Tr(tree))) = &compiled.descriptor else {
        panic!("expected a Taproot descriptor");
    };
    let scripts: Vec<_> = tree.iter_scripts().collect();
    assert_eq!(scripts.len(), 1);
    let script = scripts[0].1.encode();
    Envelope::from_tapscript(&script, 0)
        .unwrap()
        .into_iter()
        .map(|raw| {
            let parsed: Envelope<Inscription> = raw.into();
            parsed.payload
        })
        .collect()
}

struct Repeated<const IN_TEMPLATE: bool> {
    nested: bool,
}

impl<const IN_TEMPLATE: bool> Repeated<IN_TEMPLATE> {
    fn clause(&self) -> Clause {
        let inscription = effect(b"repeated");
        if self.nested {
            Clause::And(vec![Clause::Trivial, inscription])
        } else {
            inscription
        }
    }

    #[guard]
    fn inscribe(self, _ctx: Context) {
        self.clause()
    }

    #[then(guarded_by = "[Self::inscribe, Self::inscribe]")]
    fn action_guards(self, ctx: Context) {
        ctx.template()
            .add_output(Amount::from_sat(1_000), &destination(), None)?
            .into()
    }

    #[then]
    fn template_guards(self, ctx: Context) {
        ctx.template()
            .add_guard(self.clause())
            .add_guard(self.clause())
            .add_output(Amount::from_sat(1_000), &destination(), None)?
            .into()
    }
}

impl<const IN_TEMPLATE: bool> Contract for Repeated<IN_TEMPLATE> {
    const THEN_FNS: &'static [fn() -> Option<ThenFuncAsFinishOrFunc<'static, Self, ()>>] =
        if IN_TEMPLATE {
            &[Self::template_guards]
        } else {
            &[Self::action_guards]
        };
    declare! {non updatable}
}

#[test]
fn repeated_action_guards_keep_direct_and_nested_inscription_envelopes() {
    for nested in [false, true] {
        let compiled = Repeated::<false> { nested }.compile(context()).unwrap();
        assert_eq!(
            encoded_inscriptions(&compiled),
            vec![inscription(b"repeated"); 2],
            "nested={nested}"
        );
    }
}

#[test]
fn repeated_template_guards_keep_direct_and_nested_inscription_envelopes() {
    for nested in [false, true] {
        let compiled = Repeated::<true> { nested }.compile(context()).unwrap();
        assert_eq!(
            encoded_inscriptions(&compiled),
            vec![inscription(b"repeated"); 2],
            "nested={nested}"
        );
    }
}

struct Ordered;

impl Ordered {
    #[then]
    fn pay(self, ctx: Context) {
        ctx.template()
            .add_guard(effect(b"z"))
            .add_guard(effect(b"a"))
            .add_guard(effect(b"z"))
            .add_output(Amount::from_sat(1_000), &destination(), None)?
            .into()
    }
}

impl Contract for Ordered {
    declare! {then, Self::pay}
    declare! {non updatable}
}

#[test]
fn conjunction_keeps_effect_order_and_nonadjacent_duplicates() {
    let compiled = Ordered.compile(context()).unwrap();
    let template = compiled.ctv_to_tx.values().next().unwrap();
    assert_eq!(
        template.guards,
        vec![Clause::Threshold(3, vec![effect(b"z"), effect(b"a"), effect(b"z")]).into()]
    );
    let mut inscriptions = encoded_inscriptions(&compiled);
    // Miniscript chooses the final instruction order while compiling the
    // conjunction; every declared envelope must still appear in the script.
    inscriptions.sort();
    assert_eq!(
        inscriptions,
        vec![inscription(b"a"), inscription(b"z"), inscription(b"z")]
    );
}
