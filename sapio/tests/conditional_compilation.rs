use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
use bitcoin::{Amount, Network, XOnlyPublicKey};
use sapio::contract::actions::{
    ConditionalCompileType as Condition, ConditionallyCompileIf, Guard, ThenFunc,
    ThenFuncAsFinishOrFunc, ThenFuncTypeTag,
};
use sapio::contract::{empty, Compilable, CompilationError, Context, DynamicContract, TxTmplIt};
use sapio_base::covenant::LoweringPlan;
use sapio_base::Clause;
use std::cell::{Cell, RefCell};
use std::collections::LinkedList;
use std::sync::Arc;

fn failure(messages: &[&str]) -> Condition {
    Condition::Fail(messages.iter().map(|message| (*message).into()).collect())
}

fn kind(condition: &Condition) -> usize {
    match condition {
        Condition::Skippable => 0,
        Condition::Nullable => 1,
        Condition::Required => 2,
        Condition::Never => 3,
        Condition::NoConstraint => 4,
        Condition::Fail(_) => 5,
    }
}

fn cases() -> [Condition; 6] {
    [
        Condition::Skippable,
        Condition::Nullable,
        Condition::Required,
        Condition::Never,
        Condition::NoConstraint,
        failure(&["explicit error"]),
    ]
}

#[test]
fn complete_merge_truth_table_and_ordered_failure_aggregation() {
    // Rows and columns: Skippable, Nullable, Required, Never, NoConstraint, Fail.
    let expected = [
        [0, 0, 2, 3, 0, 5],
        [0, 1, 2, 3, 1, 5],
        [2, 2, 2, 5, 2, 5],
        [3, 3, 5, 3, 3, 5],
        [0, 1, 2, 3, 4, 5],
        [5, 5, 5, 5, 5, 5],
    ];
    for (row, left) in cases().iter().enumerate() {
        for (column, right) in cases().iter().enumerate() {
            let merged = left.clone().merge(right.clone());
            assert_eq!(kind(&merged), expected[row][column], "{left:?} + {right:?}");
            if let Condition::Fail(messages) = merged {
                let expected_messages = match (row, column) {
                    (5, 5) => vec!["explicit error", "explicit error"],
                    (5, _) | (_, 5) => vec!["explicit error"],
                    (2, 3) | (3, 2) => vec!["Never and Required incompatible"],
                    _ => unreachable!("only failures have messages"),
                };
                assert_eq!(
                    messages.iter().map(String::as_str).collect::<Vec<_>>(),
                    expected_messages
                );
            }
        }
    }
    assert_eq!(
        failure(&["second", "first"]).merge(failure(&["third"])),
        failure(&["second", "first", "third"])
    );
    assert_eq!(failure(&[]).merge(Condition::Nullable), failure(&[]));
}

#[test]
fn merge_decisions_are_commutative_and_associative_including_failures() {
    let mut values = cases().to_vec();
    values.extend([failure(&[]), failure(&["another error"])]);
    for left in &values {
        for middle in &values {
            assert_eq!(
                kind(&left.clone().merge(middle.clone())),
                kind(&middle.clone().merge(left.clone()))
            );
            for right in &values {
                let lhs = left.clone().merge(middle.clone()).merge(right.clone());
                let rhs = left.clone().merge(middle.clone().merge(right.clone()));
                assert_eq!(kind(&lhs), kind(&rhs), "{left:?}, {middle:?}, {right:?}");
            }
        }
    }
    // Ordered diagnostics deliberately have different algebra from decisions.
    assert_eq!(
        failure(&["first"])
            .merge(Condition::Required)
            .merge(Condition::Never),
        failure(&["first"])
    );
    assert_eq!(
        failure(&["first"]).merge(Condition::Required.merge(Condition::Never)),
        failure(&["first", "Never and Required incompatible"])
    );
}

struct State {
    conditions: [Condition; 4],
    paths: RefCell<Vec<(usize, String)>>,
    guard_calls: Cell<usize>,
    body_calls: Cell<usize>,
    empty: bool,
    body_error: bool,
}

fn key(byte: u8) -> XOnlyPublicKey {
    Keypair::from_secret_key(
        &Secp256k1::new(),
        &SecretKey::from_slice(&[byte; 32]).unwrap(),
    )
    .x_only_public_key()
    .0
}

fn context() -> Context {
    Context::new(
        Network::Regtest,
        Amount::from_sat(1000),
        LoweringPlan::Native,
        "conditional".try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    )
}

fn evaluate<const INDEX: usize>(state: &State, context: Context) -> Condition {
    state
        .paths
        .borrow_mut()
        .push((INDEX, context.path().as_ref().clone().into()));
    state.conditions[INDEX].clone()
}

fn condition<const INDEX: usize>() -> Option<ConditionallyCompileIf<State>> {
    Some(ConditionallyCompileIf::Fresh(evaluate::<INDEX>))
}

fn absent() -> Option<ConditionallyCompileIf<State>> {
    None
}

fn branch_guard() -> Option<Guard<State>> {
    Some(Guard::Fresh(
        |state, _| {
            state.guard_calls.set(state.guard_calls.get() + 1);
            Clause::Key(key(2))
        },
        None,
    ))
}

fn finish_guard() -> Option<Guard<State>> {
    Some(Guard::Fresh(|_, _| Clause::Key(key(1)), None))
}

fn body(state: &State, context: Context, _: ThenFuncTypeTag) -> TxTmplIt {
    state.body_calls.set(state.body_calls.get() + 1);
    if state.body_error {
        return Err(CompilationError::TerminateWith("body error".into()));
    }
    if state.empty {
        empty()
    } else {
        context
            .template()
            .add_output(Amount::from_sat(1000), &key(3), None)?
            .into()
    }
}

fn action() -> Option<ThenFuncAsFinishOrFunc<'static, State, ()>> {
    Some(
        ThenFunc {
            guard: &[branch_guard],
            conditional_compile_if: &[
                condition::<0>,
                condition::<1>,
                condition::<2>,
                condition::<3>,
            ],
            func: body,
            name: Arc::new("candidate".into()),
        }
        .into(),
    )
}

fn sparse_action() -> Option<ThenFuncAsFinishOrFunc<'static, State, ()>> {
    Some(
        ThenFunc {
            guard: &[branch_guard],
            conditional_compile_if: &[absent, condition::<0>, absent, condition::<1>],
            func: body,
            name: Arc::new("candidate".into()),
        }
        .into(),
    )
}

fn contract(conditions: [Condition; 4]) -> DynamicContract<'static, (), State> {
    DynamicContract {
        then: vec![action],
        finish_or: vec![],
        finish: vec![finish_guard],
        metadata_f: Box::new(|_, _| Ok(Default::default())),
        ensure_amount_f: Box::new(|_, context| Ok(context.funds())),
        data: State {
            conditions,
            paths: RefCell::new(vec![]),
            guard_calls: Cell::new(0),
            body_calls: Cell::new(0),
            empty: false,
            body_error: false,
        },
    }
}

fn single_condition(condition: Condition) -> DynamicContract<'static, (), State> {
    contract([
        condition,
        Condition::NoConstraint,
        Condition::NoConstraint,
        Condition::NoConstraint,
    ])
}

#[test]
fn absent_factories_preserve_declared_condition_context_slots() {
    let mut contract = single_condition(Condition::NoConstraint);
    contract.then = vec![sparse_action];
    let compiled = contract.compile(context()).unwrap();
    assert_eq!(
        *contract.data.paths.borrow(),
        vec![
            (0, "conditional/@action/candidate/@cond_comp_if/#1".into()),
            (1, "conditional/@action/candidate/@cond_comp_if/#3".into()),
        ]
    );
    assert_eq!(contract.data.guard_calls.get(), 1);
    assert_eq!(contract.data.body_calls.get(), 1);
    assert_eq!(compiled.ctv_to_tx.len(), 1);
    let template = compiled.ctv_to_tx.values().next().unwrap();
    assert_eq!(template.tx.output.len(), 1);
    assert_eq!(template.tx.output[0].value, 1000);
    assert_eq!(
        template.tx.output[0].script_pubkey,
        bitcoin::Script::from(&key(3).compile(context()).unwrap().address)
    );
}

#[test]
fn never_and_skippable_do_not_execute_the_guard_or_action() {
    for condition in [Condition::Never, Condition::Skippable] {
        let mut contract = single_condition(condition);
        contract.data.body_error = true;
        let compiled = contract.compile(context()).unwrap();
        assert!(compiled.ctv_to_tx.is_empty());
        assert_eq!(contract.data.guard_calls.get(), 0);
        assert_eq!(contract.data.body_calls.get(), 0);
        assert_eq!(contract.data.paths.borrow().len(), 4);
    }
}

#[test]
fn nullable_allows_no_templates_but_does_not_suppress_action_errors() {
    let mut nullable = single_condition(Condition::Nullable);
    nullable.data.empty = true;
    assert!(nullable.compile(context()).unwrap().ctv_to_tx.is_empty());
    assert_eq!(nullable.data.guard_calls.get(), 1);
    assert_eq!(nullable.data.body_calls.get(), 1);
    for condition in [Condition::Required, Condition::NoConstraint] {
        let mut required = single_condition(condition);
        required.data.empty = true;
        assert!(matches!(
            required.compile(context()),
            Err(CompilationError::MissingTemplates)
        ));
        assert_eq!(required.data.body_calls.get(), 1);
    }
    let mut error = single_condition(Condition::Nullable);
    error.data.body_error = true;
    assert!(
        matches!(error.compile(context()), Err(CompilationError::TerminateWith(message)) if message == "body error")
    );
}

#[test]
fn condition_errors_are_ordered_and_do_not_hide_required_never_conflicts() {
    for conditions in [
        [
            failure(&["second", "first"]),
            Condition::Required,
            Condition::Never,
            failure(&["third"]),
        ],
        [
            Condition::Required,
            failure(&["second", "first"]),
            failure(&["third"]),
            Condition::Never,
        ],
        [
            Condition::Never,
            Condition::Required,
            failure(&["second", "first"]),
            failure(&["third"]),
        ],
    ] {
        let contract = contract(conditions);
        let result = contract.compile(context());
        let Err(CompilationError::ConditionalCompilationFailed(messages)) = result else {
            panic!("expected conditional-compilation diagnostics");
        };
        assert_eq!(
            messages,
            [
                "second",
                "first",
                "third",
                "Never and Required incompatible"
            ]
            .into_iter()
            .map(String::from)
            .collect::<LinkedList<_>>()
        );
        assert_eq!(contract.data.paths.borrow().len(), 4);
        assert_eq!(contract.data.guard_calls.get(), 0);
        assert_eq!(contract.data.body_calls.get(), 0);
    }
    let empty_failure = single_condition(failure(&[]));
    assert!(
        matches!(empty_failure.compile(context()), Err(CompilationError::ConditionalCompilationFailed(messages)) if messages.is_empty())
    );
}

#[test]
fn repeated_contradictory_conditions_produce_one_diagnostic() {
    let contract = contract([
        Condition::Required,
        Condition::Never,
        Condition::Never,
        Condition::Required,
    ]);
    assert!(
        matches!(contract.compile(context()), Err(CompilationError::ConditionalCompilationFailed(messages))
        if messages == ["Never and Required incompatible".to_owned()].into_iter().collect())
    );
    assert_eq!(contract.data.paths.borrow().len(), 4);
    assert_eq!(contract.data.body_calls.get(), 0);
}
