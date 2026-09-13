use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
use bitcoin::{Amount, Network, XOnlyPublicKey};
use sapio::contract::actions::{ErasedAction, TemplateKind};
use sapio::contract::{Compilable, CompilationError, Context};
use sapio::template::Template;
use sapio_base::effects::{EffectPath, MapEffectDB};
use sapio_base::{Clause, LoweringPlan};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::sync::Arc;

fn key(byte: u8) -> XOnlyPublicKey {
    Keypair::from_secret_key(
        &Secp256k1::new(),
        &SecretKey::from_slice(&[byte; 32]).unwrap(),
    )
    .x_only_public_key()
    .0
}
fn path() -> EffectPath {
    "typed".try_into().unwrap()
}
fn context(effects: MapEffectDB) -> Context {
    Context::new(
        Network::Regtest,
        Amount::from_sat(1000),
        LoweringPlan::Native,
        path(),
        Arc::new(effects),
        None,
    )
}
fn payment(ctx: Context, amount: u64) -> Result<Template, CompilationError> {
    Ok(ctx
        .template()
        .add_output(Amount::from_sat(amount), &key(3), None)?
        .into())
}

// Neither request type needs Default or a contract-wide enum.
#[derive(Serialize, Deserialize, JsonSchema)]
struct Pay {
    amount: u64,
}
#[derive(Serialize, Deserialize, JsonSchema)]
struct Memo {
    text: String,
}
#[derive(Default)]
struct Wallet {
    calls: RefCell<Vec<String>>,
}

#[sapio::contract]
impl Wallet {
    #[policy]
    fn signed(&self) -> Clause {
        Clause::Key(key(1))
    }

    #[action(suggested, guarded_by(Self::signed))]
    fn pay(&self, ctx: Context, request: Pay) -> Result<Template, CompilationError> {
        self.calls
            .borrow_mut()
            .push(format!("pay:{}", request.amount));
        payment(ctx, request.amount)
    }

    #[action(suggested, guarded_by(Self::signed))]
    fn memo(&self, ctx: Context, request: Memo) -> Result<Vec<Template>, CompilationError> {
        self.calls
            .borrow_mut()
            .push(format!("memo:{}", request.text));
        Ok(vec![payment(ctx, 700)?])
    }

    #[action(suggested, guarded_by(Self::signed))]
    fn unit(&self, ctx: Context) -> Result<Template, CompilationError> {
        self.calls.borrow_mut().push("unit".into());
        payment(ctx, 600)
    }

    #[amount]
    fn balance(&self, _ctx: Context) -> Result<Amount, CompilationError> {
        Ok(Amount::from_sat(1000))
    }
}

#[test]
fn policy_discovery_does_not_fabricate_requests_and_schemas_are_per_action() {
    let wallet = Wallet::default();
    let artifact = wallet.compile(context(MapEffectDB::default())).unwrap();
    assert!(wallet.calls.borrow().is_empty());
    assert!(artifact.suggested_txs.is_empty());
    assert_eq!(artifact.continue_apis.len(), 3);
    assert_eq!(artifact.required_input_amount, Amount::from_sat(1000));
    let pay = Wallet::pay_action();
    let memo = Wallet::memo_action();
    let unit = Wallet::unit_action();
    assert!(pay.schema().unwrap()["properties"].get("amount").is_some());
    assert!(memo.schema().unwrap()["properties"].get("text").is_some());
    assert_eq!(unit.schema().unwrap()["type"], "null");
}

#[test]
fn a_typed_request_invokes_only_its_action_and_null_is_a_real_unit_request() {
    let wallet = Wallet::default();
    let request = Wallet::pay_action()
        .request(&path(), &Pay { amount: 900 })
        .unwrap();
    let artifact = wallet.compile(context(request)).unwrap();
    assert_eq!(*wallet.calls.borrow(), ["pay:900"]);
    assert_eq!(artifact.suggested_txs.len(), 1);
    wallet.calls.borrow_mut().clear();
    let request = Wallet::unit_action().request(&path(), &()).unwrap();
    wallet.compile(context(request)).unwrap();
    assert_eq!(*wallet.calls.borrow(), ["unit"]);
    wallet.calls.borrow_mut().clear();
    assert!(Wallet::memo_action()
        .call_json(
            &wallet,
            context(MapEffectDB::default()),
            serde_json::json!({"amount": 900})
        )
        .is_err());
    assert!(wallet.calls.borrow().is_empty());
}

#[test]
fn methods_remain_ordinary_rust_and_handles_keep_request_types() {
    let wallet = Wallet::default();
    let direct = wallet
        .pay(context(MapEffectDB::default()), Pay { amount: 900 })
        .unwrap();
    let typed = Wallet::pay_action()
        .invoke(
            &wallet,
            context(MapEffectDB::default()),
            Pay { amount: 900 },
        )
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(direct.tx, typed.tx);
    assert_eq!(*wallet.calls.borrow(), ["pay:900", "pay:900"]);
    assert_eq!(wallet.signed(), Clause::Key(key(1)));
}

struct Defaults {
    calls: RefCell<Vec<&'static str>>,
}
#[sapio::contract]
impl Defaults {
    #[policy]
    fn signed(&self) -> Clause {
        Clause::Key(key(1))
    }
    #[action(suggested, guarded_by(Self::signed), defaults = Self::proposals)]
    fn pay(&self, ctx: Context, request: Pay) -> Result<Template, CompilationError> {
        self.calls.borrow_mut().push("request");
        payment(ctx, request.amount)
    }
    fn proposals(&self, ctx: Context) -> Result<Vec<Template>, CompilationError> {
        self.calls.borrow_mut().push("defaults");
        Ok(vec![payment(ctx, 800)?])
    }
}
#[test]
fn default_proposals_are_explicit_and_independent_of_request_values() {
    let contract = Defaults {
        calls: RefCell::new(vec![]),
    };
    let empty = contract.compile(context(MapEffectDB::default())).unwrap();
    assert_eq!(empty.suggested_txs.len(), 1);
    assert_eq!(*contract.calls.borrow(), ["defaults"]);
    contract.calls.borrow_mut().clear();
    let request = Defaults::pay_action()
        .request(&path(), &Pay { amount: 900 })
        .unwrap();
    let requested = contract.compile(context(request)).unwrap();
    assert_eq!(requested.suggested_txs.len(), 2);
    assert_eq!(*contract.calls.borrow(), ["defaults", "request"]);
    assert_eq!(empty.address, requested.address);
}

struct Modes;
#[sapio::contract]
impl Modes {
    #[policy]
    fn signed(&self) -> Clause {
        Clause::Key(key(1))
    }
    #[action(committed, guarded_by(Self::signed))]
    fn committed(&self, ctx: Context) -> Result<Template, CompilationError> {
        payment(ctx, 700)
    }
    #[action(suggested, guarded_by(Self::signed), default)]
    fn suggested(&self, ctx: Context) -> Result<Template, CompilationError> {
        payment(ctx, 600)
    }
}
#[test]
fn commitment_mode_is_explicit_and_no_argument_committed_actions_have_defaults() {
    assert_eq!(Modes::committed_action().kind(), TemplateKind::Committed);
    assert_eq!(Modes::suggested_action().kind(), TemplateKind::Suggested);
    let artifact = Modes.compile(context(MapEffectDB::default())).unwrap();
    assert_eq!(artifact.ctv_to_tx.len(), 1);
    assert_eq!(artifact.suggested_txs.len(), 1);
    assert!(!artifact
        .ctv_to_tx
        .values()
        .next()
        .unwrap()
        .guards
        .is_empty());
    assert!(artifact
        .suggested_txs
        .values()
        .next()
        .unwrap()
        .guards
        .is_empty());
    assert_eq!(artifact.continue_apis.len(), 2);
    artifact.validate().unwrap();
}

trait Optional: sapio::contract::Contract {
    sapio::decl_then! { optional }
    sapio::decl_guard! { optional_guard }
}
struct OptionalContract;
impl Optional for OptionalContract {}
#[sapio::contract(actions(Self::optional), spends(Self::optional_guard))]
impl OptionalContract {
    #[spend]
    fn signed(&self) -> Clause {
        Clause::Key(key(2))
    }
    #[internal_key]
    fn internal(&self, _ctx: &Context) -> Result<Option<XOnlyPublicKey>, CompilationError> {
        Ok(Some(key(2)))
    }
    #[metadata]
    fn annotations(
        &self,
        _ctx: Context,
    ) -> Result<sapio::contract::object::ObjectMetadata, CompilationError> {
        Ok(Default::default())
    }
}
#[test]
fn optional_interface_exports_and_contract_hooks_remain_explicit() {
    let artifact = OptionalContract
        .compile(context(MapEffectDB::default()))
        .unwrap();
    assert!(artifact.ctv_to_tx.is_empty());
    assert!(artifact.continue_apis.is_empty());
    artifact.validate().unwrap();
}

struct Gated(bool);
#[sapio::contract]
impl Gated {
    #[spend]
    fn fallback(&self) -> Clause {
        Clause::Key(key(1))
    }
    #[condition]
    fn available(&self, _ctx: Context) -> sapio::contract::actions::ConditionalCompileType {
        use sapio::contract::actions::ConditionalCompileType;
        if self.0 {
            ConditionalCompileType::Required
        } else {
            ConditionalCompileType::Never
        }
    }
    #[action(committed, compile_if(Self::available), local)]
    fn pay(&self, ctx: Context) -> Result<Template, CompilationError> {
        payment(ctx, 700)
    }
}
#[test]
fn marked_conditions_control_presence_before_generating_transactions() {
    assert!(Gated(false)
        .compile(context(MapEffectDB::default()))
        .unwrap()
        .ctv_to_tx
        .is_empty());
    assert_eq!(
        Gated(true)
            .compile(context(MapEffectDB::default()))
            .unwrap()
            .ctv_to_tx
            .len(),
        1
    );
    assert!(Gated::pay_action().request(&path(), &()).is_err());
}

#[test]
fn multiple_requests_keep_distinct_deterministic_entries() {
    let wallet = Wallet::default();
    let candidates: Vec<_> = (700..712).map(|amount| Pay { amount }).collect();
    let effects = Wallet::pay_action().requests(&path(), &candidates).unwrap();
    let artifact = wallet.compile(context(effects)).unwrap();
    assert_eq!(
        *wallet.calls.borrow(),
        (700..712)
            .map(|amount| format!("pay:{amount}"))
            .collect::<Vec<_>>()
    );
    assert_eq!(artifact.suggested_txs.len(), 12);
}

struct PlanResult;
#[sapio::contract]
impl PlanResult {
    #[action(committed)]
    fn pay(&self, ctx: Context) -> Result<Template, sapio::template::PlanError> {
        let destination = key(3);
        let mut plan = ctx.template_plan();
        plan.output(
            "recipient",
            sapio::template::OutputAmount::Remainder,
            &destination,
        )?;
        plan.finish()
    }
}
#[test]
fn actions_accept_the_planners_own_error_type_without_return_wrappers() {
    assert_eq!(
        PlanResult
            .compile(context(MapEffectDB::default()))
            .unwrap()
            .ctv_to_tx
            .len(),
        1
    );
}

struct ConditionalMethods;
#[sapio::contract]
impl ConditionalMethods {
    #[spend]
    fn signed(&self) -> Clause {
        Clause::Key(key(1))
    }

    #[cfg(any())]
    #[action(committed)]
    fn unavailable(&self, _ctx: Context, _request: UnavailableRequest) -> UnavailableResult {
        unreachable!()
    }

    #[cfg(any())]
    #[spend]
    fn unavailable_policy(&self) -> UnavailablePolicy {
        unreachable!()
    }

    #[cfg(any())]
    #[amount]
    fn unavailable_amount(&self, _ctx: Context) -> UnavailableResult {
        unreachable!()
    }

    #[cfg_attr(all(), cfg(any()))]
    #[internal_key]
    fn unavailable_key(&self, _ctx: &Context) -> UnavailableResult {
        unreachable!()
    }

    #[cfg(any())]
    #[metadata]
    fn unavailable_metadata(&self, _ctx: Context) -> UnavailableResult {
        unreachable!()
    }
}

#[test]
fn rust_cfg_controls_exports_and_hook_delegation_with_the_original_methods() {
    let artifact = ConditionalMethods
        .compile(context(MapEffectDB::default()))
        .unwrap();
    assert!(artifact.ctv_to_tx.is_empty());
    assert!(artifact.continue_apis.is_empty());
    artifact.validate().unwrap();
}
