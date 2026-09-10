use bitcoin::secp256k1::{Secp256k1, SecretKey};
use bitcoin::{Amount, Network, XOnlyPublicKey};
use sapio::contract::actions::{CallableAsFoF, Guard};
use sapio::contract::object::SupportedDescriptors;
use sapio::contract::{Compilable, CompilationError, Compiled, Contract};
use sapio::miniscript::ord::Inscription;
use sapio::miniscript::policy::{
    compiler::CompilerError, concrete::PolicyError, semantic::Policy, Liftable,
};
use sapio::miniscript::MiniscriptKey;
use sapio::{continuation, declare, guard, then, Context};
use sapio_base::covenant::LoweringPlan;
use sapio_base::Clause;
use std::sync::Arc;

fn key(index: u8) -> XOnlyPublicKey {
    SecretKey::from_slice(&[index; 32])
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
        "validation".try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    )
}

struct SinglePolicy<const CONTINUATION: bool>(Clause);

impl<const CONTINUATION: bool> SinglePolicy<CONTINUATION> {
    #[guard]
    fn policy(self, _ctx: Context) {
        self.0.clone()
    }

    #[continuation(guarded_by = "[Self::policy]", coerce_args = "Ok")]
    fn update(self, _ctx: Context, _args: ()) {
        sapio::contract::empty()
    }
}

impl<const CONTINUATION: bool> Contract for SinglePolicy<CONTINUATION> {
    const FINISH_FNS: &'static [fn() -> Option<Guard<Self>>] =
        if CONTINUATION { &[] } else { &[Self::policy] };
    const FINISH_OR_FUNCS: &'static [fn() -> Option<Box<dyn CallableAsFoF<Self, ()>>>] =
        if CONTINUATION { &[Self::update] } else { &[] };
    declare! {non updatable}
}

fn assert_policy_error(result: Result<Compiled, CompilationError>, expected: PolicyError) {
    match result {
        Err(CompilationError::Miniscript(CompilerError::PolicyError(actual))) => {
            assert_eq!(actual, expected);
        }
        other => panic!("expected policy error {expected:?}, received {other:?}"),
    }
}

fn assert_finish_and_continuation_error(policy: Clause, expected: PolicyError) {
    assert_policy_error(
        SinglePolicy::<false>(policy.clone()).compile(context()),
        expected,
    );
    assert_policy_error(SinglePolicy::<true>(policy).compile(context()), expected);
}

#[test]
fn malformed_policy_roots_fail_before_alternatives_are_flattened() {
    let cases = [
        (Clause::And(vec![]), PolicyError::NonBinaryArgAnd),
        (
            Clause::And(vec![Clause::Key(key(1))]),
            PolicyError::NonBinaryArgAnd,
        ),
        (
            Clause::And((1..=3).map(|i| Clause::Key(key(i))).collect()),
            PolicyError::NonBinaryArgAnd,
        ),
        (Clause::Or(vec![]), PolicyError::NonBinaryArgOr),
        (
            Clause::Or(vec![(1, Clause::Key(key(1)))]),
            PolicyError::NonBinaryArgOr,
        ),
        (
            Clause::Or((1..=3).map(|i| (1, Clause::Key(key(i)))).collect()),
            PolicyError::NonBinaryArgOr,
        ),
        (Clause::Threshold(0, vec![]), PolicyError::IncorrectThresh),
        (Clause::Threshold(1, vec![]), PolicyError::IncorrectThresh),
        (
            Clause::Threshold(0, vec![Clause::Key(key(1))]),
            PolicyError::IncorrectThresh,
        ),
        (
            Clause::Threshold(2, vec![Clause::Key(key(1))]),
            PolicyError::IncorrectThresh,
        ),
    ];
    for (policy, expected) in cases {
        assert_finish_and_continuation_error(policy, expected);
    }
}

#[test]
fn malformed_nested_nodes_cannot_disappear_during_simplification() {
    let malformed = Clause::Or(vec![]);
    let inscription = Inscription::new(Some(b"text/plain".to_vec()), Some(b"body".to_vec()));
    for policy in [
        Clause::And(vec![Clause::Trivial, malformed.clone()]),
        Clause::And(vec![Clause::Unsatisfiable, malformed.clone()]),
        Clause::And(vec![malformed.clone(), Clause::Unsatisfiable]),
        Clause::Or(vec![(1, Clause::Trivial), (1, malformed.clone())]),
        Clause::Or(vec![(1, Clause::Key(key(1))), (1, malformed.clone())]),
        Clause::Threshold(1, vec![Clause::Key(key(1)), malformed.clone()]),
        Clause::Threshold(2, vec![Clause::Unsatisfiable, malformed.clone()]),
        Clause::Inscribe(Box::new(inscription), Box::new(malformed)),
    ] {
        assert_finish_and_continuation_error(policy, PolicyError::NonBinaryArgOr);
    }
}

struct ConjoinedPolicies {
    first: Clause,
    second: Clause,
}

impl ConjoinedPolicies {
    #[guard]
    fn first(self, _ctx: Context) {
        self.first.clone()
    }

    #[guard]
    fn second(self, _ctx: Context) {
        self.second.clone()
    }

    #[continuation(guarded_by = "[Self::first, Self::second]", coerce_args = "Ok")]
    fn update(self, _ctx: Context, _args: ()) {
        sapio::contract::empty()
    }
}

impl Contract for ConjoinedPolicies {
    declare! {updatable<()>, Self::update}
}

#[test]
fn every_source_guard_is_validated_before_conjunction_shortcuts() {
    for literal in [Clause::Trivial, Clause::Unsatisfiable] {
        let malformed = Clause::Threshold(1, vec![]);
        for (first, second) in [(literal.clone(), malformed.clone()), (malformed, literal)] {
            assert_policy_error(
                ConjoinedPolicies { first, second }.compile(context()),
                PolicyError::IncorrectThresh,
            );
        }
    }
}

#[test]
fn invalid_terminals_are_rejected_even_in_dead_conjunctions() {
    let invalid_inscription = Inscription::new(Some(vec![b'x'; 521]), Some(b"body".to_vec()));
    let cases = [
        (Clause::After(0), PolicyError::ZeroTime),
        (Clause::Older(0), PolicyError::ZeroTime),
        (Clause::After(u32::MAX), PolicyError::TimeTooFar),
        (Clause::Older(u32::MAX), PolicyError::TimeTooFar),
        (
            Clause::Inscribe(Box::new(invalid_inscription), Box::new(Clause::Trivial)),
            PolicyError::InvalidInscription,
        ),
    ];
    for (terminal, expected) in cases {
        assert_finish_and_continuation_error(terminal.clone(), expected);
        assert_finish_and_continuation_error(
            Clause::And(vec![Clause::Unsatisfiable, terminal.clone()]),
            expected,
        );
        for (first, second) in [
            (Clause::Unsatisfiable, terminal.clone()),
            (terminal, Clause::Unsatisfiable),
        ] {
            assert_policy_error(
                ConjoinedPolicies { first, second }.compile(context()),
                expected,
            );
        }
    }
}

struct TemplatePolicies(Vec<Clause>);

impl TemplatePolicies {
    #[then]
    fn pay(self, ctx: Context) {
        let mut builder = ctx.template();
        for policy in &self.0 {
            builder = builder.add_guard(policy.clone());
        }
        builder
            .add_output(Amount::from_sat(1_000), &key(3), None)?
            .into()
    }
}

impl Contract for TemplatePolicies {
    declare! {then, Self::pay}
    declare! {non updatable}
}

#[test]
fn template_guard_simplification_also_rejects_hidden_malformed_nodes() {
    for literal in [Clause::Trivial, Clause::Unsatisfiable] {
        let malformed = Clause::And(vec![]);
        for guards in [
            vec![literal.clone(), malformed.clone()],
            vec![malformed, literal],
        ] {
            assert_policy_error(
                TemplatePolicies(guards).compile(context()),
                PolicyError::NonBinaryArgAnd,
            );
        }
    }
}

fn accepts(policy: &Policy<XOnlyPublicKey>, signers: u8) -> bool {
    match policy {
        Policy::Unsatisfiable => false,
        Policy::Trivial => true,
        Policy::KeyHash(hash) => {
            (1..=3).any(|i| signers & (1 << (i - 1)) != 0 && key(i).to_pubkeyhash() == *hash)
        }
        Policy::Threshold(required, children) => {
            children.iter().filter(|p| accepts(p, signers)).count() >= *required
        }
        other => panic!("unexpected policy in signer fixture: {other:?}"),
    }
}

fn assert_shared_key_authorization(compiled: Compiled) {
    compiled.validate().unwrap();
    let Some(SupportedDescriptors::XOnly(descriptor)) = compiled.descriptor else {
        panic!("expected a Taproot descriptor");
    };
    let policy = descriptor.lift().unwrap();
    for signers in 0..8 {
        assert_eq!(
            accepts(&policy, signers),
            signers & 1 != 0 && signers & 6 != 0,
            "signers={signers}"
        );
    }
}

#[test]
fn separately_spendable_alternatives_can_share_a_signer() {
    let alternatives = vec![
        Clause::And(vec![Clause::Key(key(1)), Clause::Key(key(2))]),
        Clause::And(vec![Clause::Key(key(1)), Clause::Key(key(3))]),
    ];
    for policy in [
        Clause::Or(alternatives.iter().cloned().map(|p| (1, p)).collect()),
        Clause::Threshold(1, alternatives),
    ] {
        assert_shared_key_authorization(
            SinglePolicy::<false>(policy.clone())
                .compile(context())
                .unwrap(),
        );
        assert_shared_key_authorization(SinglePolicy::<true>(policy).compile(context()).unwrap());
    }
}
