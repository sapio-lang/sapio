#[path = "fixtures/covenant.rs"]
mod covenant;

use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{Secp256k1, SecretKey};
use bitcoin::util::psbt::PartiallySignedTransaction as Psbt;
use bitcoin::{Amount, Network, OutPoint, Transaction, Txid, XOnlyPublicKey};
use sapio::contract::abi::object::{ArtifactErrorKind, ObjectError};
use sapio::contract::{Compilable, CompilationError, Compiled, Context, Contract};
use sapio::{continuation, declare, guard, then};
use sapio_base::covenant::{Ctv, Emulatable, LoweringPlan};
use sapio_base::txindex::{TxIndex, TxIndexError, TxIndexLogger};
use sapio_base::Clause;
use sapio_ctv_emulator_trait::{CTVEmulator, EmulatorError};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Clone)]
enum Policy {
    Native,
    Derived(u8),
    Threshold(u8, Vec<u8>),
}

struct Emulator {
    policy: Policy,
    signing_calls: AtomicUsize,
}

impl Emulator {
    fn new(policy: Policy) -> Self {
        Self {
            policy,
            signing_calls: AtomicUsize::new(0),
        }
    }
}

impl Policy {
    fn lowering(&self) -> LoweringPlan {
        match self {
            Self::Native => LoweringPlan::Native,
            Self::Derived(seed) => covenant::plan(*seed),
            Self::Threshold(threshold, seeds) => LoweringPlan::CtvEmulation {
                signers: seeds
                    .iter()
                    .map(|seed| covenant::public_root(*seed))
                    .collect(),
                threshold: *threshold,
            },
        }
    }
}

fn owner() -> XOnlyPublicKey {
    SecretKey::from_slice(&[99; 32])
        .unwrap()
        .keypair(&Secp256k1::new())
        .x_only_public_key()
        .0
}

impl CTVEmulator for Emulator {
    fn get_signer_for(&self, hash: sha256::Hash) -> Result<Clause, EmulatorError> {
        Ok(self.policy.lowering().lower_ctv(Ctv(hash)).unwrap())
    }

    fn sign(&self, psbt: Psbt) -> Result<Psbt, EmulatorError> {
        self.signing_calls.fetch_add(1, Ordering::SeqCst);
        Ok(psbt)
    }
}

fn context(lowering: LoweringPlan, path: &str) -> Context {
    Context::new(
        Network::Regtest,
        Amount::from_sat(1_000),
        lowering,
        path.try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    )
}

struct Payment;

impl Payment {
    #[then]
    fn pay(self, ctx: Context) {
        ctx.template()
            .add_output(Amount::from_sat(1_000), &owner(), None)?
            .into()
    }
}

impl Contract for Payment {
    declare! {then, Self::pay}
    declare! {non updatable}
}

fn payment(policy: Policy) -> Compiled {
    Payment
        .compile(context(policy.lowering(), "payment"))
        .unwrap()
}

fn round_trip(object: &Compiled) -> Compiled {
    serde_json::from_slice(&serde_json::to_vec(object).unwrap()).unwrap()
}

struct NoEffects;

impl TxIndex for NoEffects {
    fn lookup_tx(&self, _: &Txid) -> Result<Arc<Transaction>, TxIndexError> {
        panic!("covenant mismatch reached funding lookup");
    }

    fn add_tx(&self, _: Arc<Transaction>) -> Result<Txid, TxIndexError> {
        panic!("covenant mismatch reached transaction insertion");
    }
}

fn assert_mismatch(object: &Compiled, conflicting: &Compiled, emulator: &Emulator) {
    let Ctv(hash) = conflicting
        .covenant_requirements
        .predicates
        .iter()
        .next()
        .unwrap();
    let expected = conflicting
        .covenant_requirements
        .lowering
        .lower_ctv(Ctv(*hash))
        .unwrap();
    let actual = emulator.get_signer_for(*hash).unwrap();
    assert_ne!(expected, actual);
    let validate = object.validate_for_emulator(emulator);
    let bind = object.bind_psbt(
        OutPoint::default(),
        BTreeMap::new(),
        Rc::new(NoEffects),
        emulator,
    );
    for error in [validate.unwrap_err(), bind.unwrap_err()] {
        let ObjectError::CovenantPolicyMismatch {
            path,
            template,
            expected: recorded,
            actual: selected,
        } = error
        else {
            panic!("unexpected error: {error}");
        };
        assert_eq!(path, conflicting.root_path);
        assert_eq!(template, *hash);
        assert_eq!(*recorded, expected);
        assert_eq!(*selected, actual);
    }
    assert_eq!(emulator.signing_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn matching_native_derived_and_threshold_policies_survive_json_and_binding() {
    for policy in [
        Policy::Native,
        Policy::Derived(1),
        Policy::Threshold(1, vec![1, 2]),
        Policy::Threshold(2, vec![1, 2]),
    ] {
        let object = round_trip(&payment(policy.clone()));
        let emulator = Emulator::new(policy.clone());
        let hash = object.ctv_to_tx.keys().next().unwrap();
        assert_eq!(object.covenant_requirements.lowering, policy.lowering());
        assert!(object
            .covenant_requirements
            .predicates
            .contains(&Ctv(*hash)));
        object.validate_for_emulator(&emulator).unwrap();
        let program = object
            .bind_psbt(
                OutPoint::default(),
                BTreeMap::new(),
                Rc::new(TxIndexLogger::new()),
                &emulator,
            )
            .unwrap();
        assert_eq!(program.program[&object.root_path].txs.len(), 1);
        assert_eq!(emulator.signing_calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn mismatched_native_keys_and_thresholds_fail_before_funding_or_signing() {
    for (compiled, selected) in [
        (Policy::Native, Policy::Derived(1)),
        (Policy::Derived(1), Policy::Native),
        (Policy::Derived(1), Policy::Derived(2)),
        (
            Policy::Threshold(1, vec![1, 2]),
            Policy::Threshold(2, vec![1, 2]),
        ),
        (
            Policy::Threshold(2, vec![1, 2]),
            Policy::Threshold(2, vec![1, 3]),
        ),
    ] {
        let object = round_trip(&payment(compiled));
        assert_mismatch(&object, &object, &Emulator::new(selected));
    }
}

struct SuggestChild(Compiled);

impl SuggestChild {
    #[guard]
    fn signed(self, _ctx: Context) {
        Clause::Key(owner())
    }

    #[continuation(guarded_by = "[Self::signed]", coerce_args = "Ok")]
    fn suggest(self, ctx: Context, _args: ()) {
        ctx.template()
            .add_output(Amount::from_sat(1_000), &self.0, None)?
            .into()
    }
}

impl Contract for SuggestChild {
    declare! {updatable<()>, Self::suggest}
}

#[test]
fn committed_descendants_are_checked_through_suggested_parent_templates() {
    let child = payment(Policy::Derived(1));
    let parent = SuggestChild(child.clone())
        .compile(context(Policy::Derived(1).lowering(), "suggestion"))
        .unwrap();
    let parent = round_trip(&parent);
    assert!(parent.ctv_to_tx.is_empty());
    assert!(parent.covenant_requirements.predicates.is_empty());
    parent
        .validate_for_emulator(&Emulator::new(Policy::Derived(1)))
        .unwrap();
    assert_mismatch(&parent, &child, &Emulator::new(Policy::Derived(2)));
}

struct NeverQuery;

impl CTVEmulator for NeverQuery {
    fn get_signer_for(&self, _: sha256::Hash) -> Result<Clause, EmulatorError> {
        panic!("ordinary destination has no covenant to compare");
    }

    fn sign(&self, _: Psbt) -> Result<Psbt, EmulatorError> {
        panic!("ordinary destination has no template to sign");
    }
}

#[test]
fn destinations_without_committed_templates_do_not_require_an_emulator_policy() {
    let object = owner()
        .compile(context(LoweringPlan::Native, "owner"))
        .unwrap();
    object.validate_for_emulator(&NeverQuery).unwrap();
    object
        .bind_psbt(
            OutPoint::default(),
            BTreeMap::new(),
            Rc::new(TxIndexLogger::new()),
            &NeverQuery,
        )
        .unwrap();
}

struct ChangingPolicy(AtomicUsize);

impl CTVEmulator for ChangingPolicy {
    fn get_signer_for(&self, hash: sha256::Hash) -> Result<Clause, EmulatorError> {
        let call = self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Clause::Key(covenant::key(
            if call == 0 { 1 } else { 2 },
            hash,
        )))
    }

    fn sign(&self, _: Psbt) -> Result<Psbt, EmulatorError> {
        panic!("changed covenant must fail before signing");
    }
}

#[test]
fn compilation_uses_only_the_explicit_plan_and_binding_detects_changed_signer_policy() {
    let emulator = ChangingPolicy(AtomicUsize::new(0));
    let object = payment(Policy::Derived(1));
    let repeated = payment(Policy::Derived(1));
    assert_eq!(object, repeated);
    // Runtime callbacks are absent from Context. Neither policy lookup nor
    // signing can participate in compilation, even for a stateful connection.
    assert_eq!(emulator.0.load(Ordering::SeqCst), 0);
    object.validate_for_emulator(&emulator).unwrap();
    assert_eq!(emulator.0.load(Ordering::SeqCst), 1);
    assert!(matches!(
        object.validate_for_emulator(&emulator),
        Err(ObjectError::CovenantPolicyMismatch { .. })
    ));
    assert_eq!(emulator.0.load(Ordering::SeqCst), 2);
}

fn assert_invalid_artifact_before_effects(object: &Compiled, expected: ArtifactErrorKind) {
    for error in [
        object.validate_for_emulator(&NeverQuery).unwrap_err(),
        object
            .bind_psbt(
                OutPoint::default(),
                BTreeMap::new(),
                Rc::new(NoEffects),
                &NeverQuery,
            )
            .unwrap_err(),
    ] {
        let ObjectError::InvalidArtifact(error) = error else {
            panic!("unexpected error: {error}");
        };
        assert_eq!(error.path, object.root_path);
        assert!(error.template.is_some());
        assert_eq!(error.kind, expected);
    }
}

#[test]
fn missing_committed_predicates_cannot_reach_signers_or_funding() {
    let mut object = payment(Policy::Native);
    object.covenant_requirements.predicates.clear();
    assert_invalid_artifact_before_effects(&object, ArtifactErrorKind::MissingCovenant);
}

#[test]
fn serialized_artifacts_require_explicit_covenant_requirements() {
    let original = payment(Policy::Native);
    let mut json = serde_json::to_value(&original).unwrap();
    json.as_object_mut()
        .unwrap()
        .remove("covenant_requirements");
    let error = serde_json::from_value::<Compiled>(json).unwrap_err();
    assert!(error.to_string().contains("covenant_requirements"));
}

#[test]
fn invalid_public_lowering_plans_fail_before_runtime_or_funding_callbacks() {
    let mut deep_root = covenant::public_root(1);
    deep_root.depth = 247;
    for lowering in [
        LoweringPlan::CtvEmulation {
            signers: vec![],
            threshold: 1,
        },
        LoweringPlan::CtvEmulation {
            signers: vec![covenant::public_root(1)],
            threshold: 0,
        },
        LoweringPlan::CtvEmulation {
            signers: vec![covenant::public_root(1), covenant::public_root(1)],
            threshold: 2,
        },
        LoweringPlan::CtvEmulation {
            signers: vec![deep_root],
            threshold: 1,
        },
    ] {
        let expected = lowering.validate().unwrap_err();
        let mut object = payment(Policy::Native);
        // Every sealed compilation entrypoint must reject invalid public input,
        // including key-only destinations and reuse of a valid artifact.
        for result in [
            Payment.compile(context(lowering.clone(), "invalid_contract")),
            owner().compile(context(lowering.clone(), "invalid_key")),
            object.compile(context(lowering.clone(), "invalid_reuse")),
        ] {
            let error = match result.unwrap_err() {
                CompilationError::Covenant(error)
                | CompilationError::CompiledObjectError(ObjectError::Covenant(error)) => error,
                error => panic!("unexpected error: {error}"),
            };
            assert_eq!(error, expected);
        }
        object.covenant_requirements.lowering = lowering;
        let object = round_trip(&object);
        for error in [
            object.validate_for_emulator(&NeverQuery).unwrap_err(),
            object
                .bind_psbt(
                    OutPoint::default(),
                    BTreeMap::new(),
                    Rc::new(NoEffects),
                    &NeverQuery,
                )
                .unwrap_err(),
        ] {
            let ObjectError::InvalidArtifact(error) = error else {
                panic!("unexpected error: {error}");
            };
            assert_eq!(error.path, object.root_path);
            assert_eq!(error.template, None);
            assert_eq!(
                error.kind,
                ArtifactErrorKind::InvalidCovenantLowering(expected.to_string())
            );
        }
    }
}

struct FailingPolicy;

impl CTVEmulator for FailingPolicy {
    fn get_signer_for(&self, _: sha256::Hash) -> Result<Clause, EmulatorError> {
        Err(EmulatorError::NetworkIssue(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "policy unavailable",
        )))
    }

    fn sign(&self, _: Psbt) -> Result<Psbt, EmulatorError> {
        panic!("failed covenant lookup reached signing");
    }
}

#[test]
fn backend_policy_errors_propagate_before_funding_or_signing() {
    let object = payment(Policy::Native);
    for error in [
        object.validate_for_emulator(&FailingPolicy).unwrap_err(),
        object
            .bind_psbt(
                OutPoint::default(),
                BTreeMap::new(),
                Rc::new(NoEffects),
                &FailingPolicy,
            )
            .unwrap_err(),
    ] {
        let ObjectError::Emulator(EmulatorError::NetworkIssue(error)) = error else {
            panic!("unexpected error: {error}");
        };
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "policy unavailable");
    }
}

#[test]
fn reused_compiled_children_reject_a_different_context_lowering_at_add_output() {
    let child = payment(Policy::Derived(1));
    let hash = child.ctv_to_tx.keys().next().unwrap();
    let result = context(LoweringPlan::Native, "parent")
        .template()
        .add_output(Amount::from_sat(1_000), &child, None);
    let error = result.err().expect("mismatched child was accepted");
    let CompilationError::CompiledObjectError(ObjectError::CovenantPolicyMismatch {
        path,
        template: actual_hash,
        expected,
        actual,
    }) = error
    else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(path, child.root_path);
    assert_eq!(actual_hash, *hash);
    assert_eq!(
        *expected,
        child
            .covenant_requirements
            .lowering
            .lower_ctv(Ctv(*hash))
            .unwrap()
    );
    assert_eq!(*actual, Clause::TxTemplate(*hash));
    assert!(context(Policy::Derived(1).lowering(), "parent")
        .template()
        .add_output(Amount::from_sat(1_000), &child, None)
        .is_ok());
}

#[test]
fn compiled_reuse_preserves_equal_and_equivalent_plans_without_skipping_validation() {
    let child = round_trip(&payment(Policy::Derived(1)));
    let plan = child.covenant_requirements.lowering.clone();
    let mut relabeled = plan.clone();
    let LoweringPlan::CtvEmulation { signers, .. } = &mut relabeled else {
        unreachable!();
    };
    signers[0].network = Network::Bitcoin;
    signers[0].depth = 10;
    assert_ne!(plan, relabeled);
    for selected in [plan.clone(), relabeled] {
        // Different descriptive xpub metadata still derives the exact same
        // signer clause. Reuse must preserve the originally compiled artifact.
        let reused = child.compile(context(selected, "reused")).unwrap();
        assert_eq!(reused, child);
    }

    let mut invalid = child;
    invalid.ctv_to_tx.values_mut().next().unwrap().ctv_index = 1;
    assert!(matches!(
        invalid.compile(context(plan, "invalid_reuse")),
        Err(CompilationError::InvalidArtifact(error))
            if error.kind == ArtifactErrorKind::UnsupportedInputIndex(1)
    ));
}

struct WrappedFinish(Ctv);

impl WrappedFinish {
    #[guard(policy, cached)]
    fn covenant(self) -> Emulatable<Ctv> {
        Emulatable(self.0)
    }
}

impl Contract for WrappedFinish {
    declare! {finish, Self::covenant}
    declare! {non updatable}
}

#[test]
fn wrapped_finish_guards_record_their_predicate_without_a_transaction_template() {
    let predicate = Ctv(sha256::Hash::hash(b"explicit finish predicate"));
    for policy in [Policy::Native, Policy::Derived(1)] {
        let compiled = WrappedFinish(predicate)
            .compile(context(policy.lowering(), "wrapped_finish"))
            .unwrap();
        let compiled = round_trip(&compiled);
        assert!(compiled.ctv_to_tx.is_empty());
        assert!(compiled.suggested_txs.is_empty());
        assert_eq!(compiled.covenant_requirements.lowering, policy.lowering());
        assert_eq!(
            compiled.covenant_requirements.predicates,
            std::collections::BTreeSet::from([predicate])
        );
        compiled
            .validate_for_emulator(&Emulator::new(policy.clone()))
            .unwrap();
        assert_eq!(
            compiled.requires_native_ctv(),
            matches!(policy, Policy::Native)
        );
        assert_mismatch(&compiled, &compiled, &Emulator::new(Policy::Derived(2)));
    }
}

struct NativeFinish(Ctv);

impl NativeFinish {
    #[guard(cached)]
    fn covenant(self) {
        Clause::TxTemplate(self.0 .0)
    }
}

impl Contract for NativeFinish {
    declare! {finish, Self::covenant}
    declare! {non updatable}
}

#[test]
fn an_unwrapped_native_guard_keeps_its_native_meaning_under_an_emulation_plan() {
    let predicate = Ctv(sha256::Hash::hash(b"native finish predicate"));
    let native = NativeFinish(predicate)
        .compile(context(LoweringPlan::Native, "native_finish"))
        .unwrap();
    let selected = NativeFinish(predicate)
        .compile(context(Policy::Derived(1).lowering(), "native_finish"))
        .unwrap();
    assert_eq!(selected.descriptor, native.descriptor);
    assert_eq!(selected.address, native.address);
    assert!(selected.requires_native_ctv());
    assert!(selected.covenant_requirements.predicates.is_empty());
    selected.validate_for_emulator(&NeverQuery).unwrap();
}

struct WrappedContinuation {
    predicate: Ctv,
    add_effect_guard: bool,
}

impl WrappedContinuation {
    #[guard(policy, cached)]
    fn covenant(self) -> Emulatable<Ctv> {
        Emulatable(self.predicate)
    }

    #[continuation(guarded_by = "[Self::covenant]", coerce_args = "Ok", web_api)]
    fn suggest(self, ctx: Context, amount: Option<u64>) {
        let mut template =
            ctx.template()
                .add_output(Amount::from_sat(amount.unwrap_or(1_000)), &owner(), None)?;
        if self.add_effect_guard && amount.is_some() {
            template = template.add_guard(Clause::Key(owner()));
        }
        template.into()
    }
}

impl Contract for WrappedContinuation {
    declare! {updatable<Option<u64>>, Self::suggest}
}

fn continuation_context(with_effect: bool) -> Context {
    let effects = if with_effect {
        serde_json::from_value(serde_json::json!({
            "effects": {"updates/@action/suggest/@suggested": {"smaller": 900}}
        }))
        .unwrap()
    } else {
        Default::default()
    };
    Context::new(
        Network::Regtest,
        Amount::from_sat(1_000),
        Policy::Derived(1).lowering(),
        "updates".try_into().unwrap(),
        Arc::new(effects),
        None,
    )
}

#[test]
fn continuation_effects_change_suggestions_without_changing_the_declared_predicate() {
    let contract = WrappedContinuation {
        predicate: Ctv(sha256::Hash::hash(b"fixed continuation predicate")),
        add_effect_guard: false,
    };
    let original = contract.compile(continuation_context(false)).unwrap();
    let updated = round_trip(&contract.compile(continuation_context(true)).unwrap());
    assert_eq!(original.descriptor, updated.descriptor);
    assert_eq!(original.address, updated.address);
    assert_eq!(
        original.covenant_requirements,
        updated.covenant_requirements
    );
    assert!(updated.ctv_to_tx.is_empty());
    assert_eq!(original.suggested_txs.len(), 1);
    assert_eq!(updated.suggested_txs.len(), 2);
    assert_eq!(
        updated.covenant_requirements.predicates,
        std::collections::BTreeSet::from([contract.predicate])
    );
    assert_mismatch(&updated, &updated, &Emulator::new(Policy::Derived(2)));

    let guarded_effect = WrappedContinuation {
        add_effect_guard: true,
        ..contract
    };
    assert!(matches!(
        guarded_effect.compile(continuation_context(true)),
        Err(CompilationError::AdditionalGuardsNotAllowedHere)
    ));
}
