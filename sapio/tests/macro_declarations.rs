use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
use bitcoin::{Amount, Network, XOnlyPublicKey};
use sapio::contract::actions::{ConditionalCompileType, Guard};
use sapio::contract::{Compilable, CompilationError, Context, Contract, TxTmplIt};
use sapio_base::effects::EffectPath;
use sapio_base::Clause;
use sapio_ctv_emulator_trait::CTVAvailable;
use std::sync::Arc;

fn context() -> Context {
    Context::new(
        Network::Regtest,
        Amount::from_sat(1_000),
        Arc::new(CTVAvailable),
        EffectPath::try_from("macro_declarations").unwrap(),
        Arc::new(Default::default()),
        None,
    )
}

fn key(byte: u8) -> XOnlyPublicKey {
    Keypair::from_secret_key(
        &Secp256k1::new(),
        &SecretKey::from_slice(&[byte; 32]).unwrap(),
    )
    .x_only_public_key()
    .0
}

struct OfflineArgs(u64);

trait Actions: Contract {
    sapio::decl_guard! { signed }
    sapio::decl_guard! { cached cached_signed }
    sapio::decl_compile_if! { allowed }
    sapio::decl_then! { pay }
    sapio::decl_continuation! { <web={}> update<u64> }
    sapio::decl_continuation! { offline<OfflineArgs> }
}

struct Policy;

impl Actions for Policy {
    #[sapio::guard]
    fn signed(&self, _ctx: Context) -> Clause {
        Clause::Key(key(1))
    }

    #[sapio::guard(cached)]
    fn cached_signed(self) {
        Clause::Key(key(2))
    }

    #[sapio::compile_if]
    fn allowed(self, _ctx: Context) -> ConditionalCompileType {
        ConditionalCompileType::Required
    }

    #[sapio::then(
        guarded_by = "[Self::signed, Self::cached_signed]",
        compile_if = "[Self::allowed]"
    )]
    fn pay(self, ctx: Context) -> TxTmplIt {
        ctx.template()
            .add_output(Amount::from_sat(1_000), &key(1), None)?
            .into()
    }

    #[sapio::continuation(
        guarded_by = "[Self::signed]",
        compile_if = "[Self::allowed]",
        coerce_args = "|value: Option<u64>| Ok(value.unwrap_or(900))",
        web_api
    )]
    fn update(self, ctx: Context, amount: u64) -> TxTmplIt {
        ctx.template()
            .add_output(Amount::from_sat(amount), &key(1), None)?
            .into()
    }

    #[sapio::continuation(
        guarded_by = "[Self::cached_signed]",
        coerce_args = "|value: Option<u64>| Ok(OfflineArgs(value.unwrap_or(800)))"
    )]
    fn offline(self, ctx: Context, OfflineArgs(amount): OfflineArgs) {
        ctx.template()
            .add_output(Amount::from_sat(amount), &key(2), None)?
            .into()
    }
}

impl Contract for Policy {
    sapio::declare! {then, Self::pay}
    sapio::declare! {updatable<Option<u64>>, Self::update, Self::offline}
}

#[test]
fn declared_traits_and_attribute_implementations_compile_real_actions() {
    assert!(matches!(Policy::signed(), Some(Guard::Fresh(..))));
    assert!(matches!(Policy::cached_signed(), Some(Guard::Cache(..))));
    let artifact = Policy.compile(context()).unwrap();
    artifact.validate().unwrap();
    assert_eq!(artifact.ctv_to_tx.len(), 1);
    assert_eq!(artifact.suggested_txs.len(), 2);
    assert_eq!(artifact.continue_apis.len(), 2);

    let update = Policy::update().unwrap();
    assert!(update.web_api());
    assert!(update.get_schema().is_some());
    let templates = update
        .call_json(&Policy, context(), serde_json::json!(700))
        .unwrap();
    let template = templates.into_iter().next().unwrap().unwrap();
    assert_eq!(template.tx.output[0].value, 700);
    assert!(update
        .call_json(&Policy, context(), serde_json::json!("bad"))
        .is_err());

    let offline = Policy::offline().unwrap();
    assert!(!offline.web_api());
    assert!(offline.get_schema().is_none());
    assert!(matches!(
        offline.call_json(&Policy, context(), serde_json::json!(800)),
        Err(CompilationError::WebAPIDisabled)
    ));
}

struct Absent;
impl Actions for Absent {}
impl Contract for Absent {
    sapio::declare! {non updatable}
}

#[test]
fn interface_defaults_leave_actions_absent() {
    assert!(Absent::signed().is_none());
    assert!(Absent::cached_signed().is_none());
    assert!(Absent::allowed().is_none());
    assert!(Absent::pay().is_none());
    assert!(Absent::update().is_none());
    assert!(Absent::offline().is_none());
}

#[allow(non_snake_case)]
trait CaseSensitive: Contract {
    sapio::decl_guard! { cached signed }
    sapio::decl_continuation! { <web={}> update<u64> }
    sapio::decl_continuation! { <web={}> UPDATE<String> }
    sapio::decl_continuation! { <web={}> r#type<bool> }
}

struct Cases;
impl Contract for Cases {
    sapio::declare! {updatable<()>, Self::update, Self::UPDATE, Self::r#type}
}

#[allow(non_snake_case)]
impl CaseSensitive for Cases {
    #[sapio::guard(cached)]
    fn signed(self) {
        Clause::Key(key(3))
    }

    #[sapio::continuation(guarded_by = "[Self::signed]", coerce_args = "|()| Ok(1)", web_api)]
    fn update(self, _ctx: Context, _value: u64) {
        sapio::contract::empty()
    }

    #[sapio::continuation(
        guarded_by = "[Self::signed]",
        coerce_args = "|()| Ok(String::new())",
        web_api
    )]
    fn UPDATE(self, _ctx: Context, _value: String) {
        sapio::contract::empty()
    }

    #[sapio::continuation(guarded_by = "[Self::signed]", coerce_args = "|()| Ok(true)", web_api)]
    fn r#type(self, _ctx: Context, _value: bool) {
        sapio::contract::empty()
    }
}

#[test]
fn continuation_schema_helpers_distinguish_case_and_support_raw_names() {
    for (action, expected) in [
        (Cases::update(), "integer"),
        (Cases::UPDATE(), "string"),
        (Cases::r#type(), "boolean"),
    ] {
        assert_eq!(
            action.unwrap().get_schema().as_ref().unwrap()["type"],
            expected
        );
    }
    assert_eq!(Cases::r#type().unwrap().get_name().as_str(), "type");
    let artifact = Cases.compile(context()).unwrap();
    artifact.validate().unwrap();
    let paths: std::collections::BTreeSet<_> = artifact
        .continue_apis
        .keys()
        .map(|path| String::from(path.0.as_ref().clone()))
        .collect();
    assert_eq!(
        paths,
        std::collections::BTreeSet::from([
            "macro_declarations/@action/update/@suggested".into(),
            "macro_declarations/@action/UPDATE/@suggested".into(),
            "macro_declarations/@action/type/@suggested".into(),
        ])
    );
}

#[allow(dead_code)]
mod hygiene {
    use ::sapio as language;

    // Generated code must resolve its own dependencies despite local names.
    mod sapio {}
    mod std {}
    struct Option;
    struct Box;

    pub struct Visible;
    impl Visible {
        /// The visibility and must-use annotation apply to the action factory.
        #[::sapio::guard(cached)]
        #[must_use]
        pub fn signed(self) -> ::sapio::sapio_base::Clause {
            ::sapio::sapio_base::Clause::Key(super::key(3))
        }
    }
    impl language::contract::Contract for Visible {
        language::declare! {non updatable}
        language::declare! {finish, Self::signed}
    }
}

#[test]
fn generated_actions_preserve_visibility_and_resolve_qualified_declarations() {
    assert!(hygiene::Visible::signed().is_some());
    hygiene::Visible
        .compile(context())
        .unwrap()
        .validate()
        .unwrap();
}
