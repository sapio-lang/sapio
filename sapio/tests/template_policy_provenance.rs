#[path = "fixtures/custom_policy.rs"]
mod fixture;

use bitcoin::blockdata::opcodes::all::OP_VERIFY;
use bitcoin::{Amount, ScriptBuf};
use fixture::{context, key, ArithmeticSigner};
use sapio::contract::actions::ActionFactory;
use sapio::contract::object::SupportedDescriptors;
use sapio::contract::{Compilable, Context, Contract, TxTmplIt};
use sapio::{guard, then};
use sapio_base::miniscript::Tap;
use sapio_base::policy::{PolicyCompiler, ScriptPolicy};
use sapio_base::Clause;
use std::collections::BTreeSet;

fn payment(ctx: Context) -> TxTmplIt {
    ctx.template()
        .add_output(Amount::from_sat(1_000), &key(9), None)?
        .into()
}

struct Payments<const REVERSED: bool>;

impl<const REVERSED: bool> Payments<REVERSED> {
    #[guard(policy)]
    fn owner(self, _ctx: Context) -> ArithmeticSigner {
        ArithmeticSigner(key(1))
    }

    #[then]
    fn unguarded(self, ctx: Context) {
        payment(ctx)
    }

    #[then(guarded_by = "[Self::owner]")]
    fn guarded(self, ctx: Context) {
        payment(ctx)
    }
}

impl<const REVERSED: bool> Contract for Payments<REVERSED> {
    const ACTIONS: &'static [ActionFactory<Self>] = if REVERSED {
        &[Self::guarded, Self::unguarded]
    } else {
        &[Self::unguarded, Self::guarded]
    };
}

#[test]
fn an_unguarded_duplicate_preserves_the_optional_raw_policy_and_its_script() {
    let forward = Payments::<false>.compile(context(false)).unwrap();
    let reverse = Payments::<true>.compile(context(false)).unwrap();
    assert_eq!(forward.ctv_to_tx.len(), 1);
    assert_eq!(forward.ctv_to_tx, reverse.ctv_to_tx);
    assert_eq!(forward.descriptor, reverse.descriptor);
    forward.validate().unwrap();
    let template = forward.ctv_to_tx.values().next().unwrap();
    let optional = ArithmeticSigner(key(1)).compile_policy().unwrap();
    assert_eq!(
        template.guards,
        vec![ScriptPolicy::Or(vec![
            Clause::Trivial.into(),
            optional.clone(),
        ])]
    );
    // An unknown witness bound does not prevent compilation when the contract
    // makes no minimum-feerate claim.
    assert_eq!(template.min_feerate_sats_vbyte, None);

    let Some(SupportedDescriptors::Taproot(raw)) = &forward.descriptor else {
        panic!("the raw authorization must retain its script spending data");
    };
    let covenant = Clause::TxTemplate(template.hash())
        .compile::<Tap>()
        .unwrap()
        .encode();
    let ScriptPolicy::Script(fragment) = optional else {
        unreachable!()
    };
    let mut guarded = fragment.into_script().as_bytes().to_vec();
    guarded.push(OP_VERIFY.to_u8());
    guarded.extend_from_slice(covenant.as_bytes());
    assert_eq!(
        raw.leaves()
            .iter()
            .map(|(_, script)| script.clone())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([covenant, ScriptBuf::from(guarded)])
    );
}
