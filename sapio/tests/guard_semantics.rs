use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
use bitcoin::{Amount, Network, XOnlyPublicKey};
use sapio::contract::{Compilable, CompilationError, Context, Contract};
use sapio::{continuation, declare, guard, then};
use sapio_base::covenant::LoweringPlan;
use sapio_base::effects::EffectPath;
use sapio_base::miniscript::policy::Liftable;
use sapio_base::miniscript::Descriptor;
use sapio_base::simp::{GuardLT, SIMPAttachableAt, SIMP};
use sapio_base::Clause;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::sync::Arc;

const PROTOCOL: i64 = -17;

fn key(byte: u8) -> XOnlyPublicKey {
    let secp = Secp256k1::new();
    Keypair::from_secret_key(&secp, &SecretKey::from_slice(&[byte; 32]).unwrap())
        .x_only_public_key()
        .0
}

enum Annotation {
    Json(Value),
    Invalid,
}

impl SIMP for Annotation {
    fn static_get_protocol_number() -> i64 {
        PROTOCOL
    }

    fn get_protocol_number(&self) -> i64 {
        PROTOCOL
    }

    fn to_json(&self) -> Result<Value, serde_json::Error> {
        match self {
            Self::Json(value) => Ok(value.clone()),
            Self::Invalid => Err(serde_json::Error::io(std::io::Error::other(
                "guard annotation cannot serialize",
            ))),
        }
    }

    fn from_json(value: Value) -> Result<Self, serde_json::Error> {
        Ok(Self::Json(value))
    }
}

impl SIMPAttachableAt<GuardLT> for Annotation {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MetadataMode {
    Paths,
    RepeatedValues,
    FailCachedFinish,
    FailFreshFinish,
    InvalidCachedFinish,
}

struct ObservedGuards {
    cached_key: XOnlyPublicKey,
    fresh_key: XOnlyPublicKey,
    cached_calls: Cell<usize>,
    fresh_contexts: RefCell<Vec<String>>,
    cached_metadata_paths: RefCell<Vec<String>>,
    fresh_metadata_paths: RefCell<Vec<String>>,
    metadata_mode: MetadataMode,
}

impl ObservedGuards {
    fn new(metadata_mode: MetadataMode) -> Self {
        Self {
            cached_key: key(1),
            fresh_key: key(2),
            cached_calls: Cell::new(0),
            fresh_contexts: RefCell::new(vec![]),
            cached_metadata_paths: RefCell::new(vec![]),
            fresh_metadata_paths: RefCell::new(vec![]),
            metadata_mode,
        }
    }

    #[guard(cached, simps = "Some(Self::cached_metadata)")]
    fn cached(self) {
        self.cached_calls.set(self.cached_calls.get() + 1);
        Clause::Key(self.cached_key)
    }

    #[guard(simps = "Some(Self::fresh_metadata)")]
    fn fresh(self, ctx: Context) {
        self.fresh_contexts
            .borrow_mut()
            .push(String::from(ctx.path().as_ref().clone()));
        Clause::Key(self.fresh_key)
    }

    fn cached_metadata(
        &self,
        ctx: Context,
    ) -> Result<Vec<Arc<dyn SIMPAttachableAt<GuardLT>>>, CompilationError> {
        let path = String::from(ctx.path().as_ref().clone());
        self.cached_metadata_paths.borrow_mut().push(path.clone());
        if path.contains("/@finish_fn/") {
            match self.metadata_mode {
                MetadataMode::FailCachedFinish => {
                    return Err(CompilationError::TerminateWith(
                        "cached finish metadata failed".into(),
                    ));
                }
                MetadataMode::InvalidCachedFinish => {
                    return Ok(vec![Arc::new(Annotation::Invalid)])
                }
                _ => {}
            }
        }
        Ok(self.annotations(path))
    }

    fn fresh_metadata(
        &self,
        ctx: Context,
    ) -> Result<Vec<Arc<dyn SIMPAttachableAt<GuardLT>>>, CompilationError> {
        let path = String::from(ctx.path().as_ref().clone());
        self.fresh_metadata_paths.borrow_mut().push(path.clone());
        if path.contains("/@finish_fn/") && self.metadata_mode == MetadataMode::FailFreshFinish {
            return Err(CompilationError::TerminateWith(
                "fresh finish metadata failed".into(),
            ));
        }
        Ok(self.annotations(path))
    }

    fn annotations(&self, path: String) -> Vec<Arc<dyn SIMPAttachableAt<GuardLT>>> {
        let values = if self.metadata_mode == MetadataMode::RepeatedValues {
            vec![
                json!("first"),
                json!("first"),
                json!("second"),
                json!("first"),
            ]
        } else {
            vec![json!({"path": path})]
        };
        values
            .into_iter()
            .map(|value| Arc::new(Annotation::Json(value)) as Arc<dyn SIMPAttachableAt<GuardLT>>)
            .collect()
    }

    #[then(guarded_by = "[Self::cached, Self::fresh]")]
    fn pay(self, ctx: Context) {
        ctx.template()
            .add_output(Amount::from_sat(1_000), &self.cached_key, None)?
            .into()
    }

    #[continuation(guarded_by = "[Self::cached, Self::fresh]", web_api)]
    fn suggest(self, _ctx: Context, _args: Option<()>) {
        sapio::contract::empty()
    }
}

impl Contract for ObservedGuards {
    declare! {actions, Self::pay, Self::suggest}
    declare! {finish, Self::cached, Self::fresh}
}

fn context() -> Context {
    Context::new(
        Network::Regtest,
        Amount::from_sat(1_000),
        LoweringPlan::Native,
        EffectPath::try_from("guards").unwrap(),
        Arc::new(Default::default()),
        None,
    )
}

#[test]
fn policies_and_metadata_use_their_declared_lifetimes_and_contexts() {
    let contract = ObservedGuards::new(MetadataMode::Paths);
    let compiled = contract.compile(context()).unwrap();
    compiled.validate().unwrap();
    assert_eq!(contract.cached_calls.get(), 1);
    assert_eq!(
        *contract.fresh_contexts.borrow(),
        [
            "guards/@action/pay/@guard/#1",
            "guards/@action/suggest/@guard/#1",
            "guards/@finish_fn/#1",
        ]
    );
    for (key, recorded, index) in [
        (contract.cached_key, &contract.cached_metadata_paths, 0),
        (contract.fresh_key, &contract.fresh_metadata_paths, 1),
    ] {
        let paths = vec![
            format!("guards/@action/pay/@guard/#{index}/@metadata"),
            format!("guards/@action/suggest/@guard/#{index}/@metadata"),
            format!("guards/@finish_fn/#{index}/@metadata"),
        ];
        assert_eq!(*recorded.borrow(), paths);
        assert_eq!(
            compiled
                .metadata
                .simps_for_guards
                .iter()
                .find(|record| record.policy == Clause::Key(key).into())
                .unwrap()
                .protocols[&PROTOCOL],
            paths
                .into_iter()
                .map(|path| json!({"path": path}))
                .collect::<Vec<_>>()
        );
    }

    let repeated = contract.compile(context()).unwrap();
    assert_eq!(contract.cached_calls.get(), 2);
    assert_eq!(contract.fresh_contexts.borrow().len(), 6);
    assert_eq!(
        serde_json::to_value(compiled).unwrap(),
        serde_json::to_value(repeated).unwrap()
    );
}

#[test]
fn equal_json_annotations_deduplicate_without_losing_distinct_values() {
    let contract = ObservedGuards::new(MetadataMode::RepeatedValues);
    let compiled = contract.compile(context()).unwrap();
    for key in [contract.cached_key, contract.fresh_key] {
        assert_eq!(
            compiled
                .metadata
                .simps_for_guards
                .iter()
                .find(|record| record.policy == Clause::Key(key).into())
                .unwrap()
                .protocols[&PROTOCOL],
            vec![json!("first"), json!("second")]
        );
    }
    assert_eq!(contract.cached_metadata_paths.borrow().len(), 3);
    assert_eq!(contract.fresh_metadata_paths.borrow().len(), 3);
}

#[test]
fn errors_at_later_metadata_attachments_abort_compilation() {
    for (mode, expected) in [
        (
            MetadataMode::FailCachedFinish,
            "cached finish metadata failed",
        ),
        (
            MetadataMode::FailFreshFinish,
            "fresh finish metadata failed",
        ),
    ] {
        let contract = ObservedGuards::new(mode);
        assert!(matches!(
            contract.compile(context()),
            Err(CompilationError::TerminateWith(message)) if message == expected
        ));
        assert_eq!(contract.cached_calls.get(), 1);
    }
}

#[test]
fn finish_metadata_serialization_errors_are_not_discarded() {
    let contract = ObservedGuards::new(MetadataMode::InvalidCachedFinish);
    assert!(matches!(
        contract.compile(context()),
        Err(CompilationError::SerializationError(error))
            if error.to_string().contains("guard annotation cannot serialize")
    ));
}

struct ManyGuards {
    keys: [XOnlyPublicKey; 4],
}

impl ManyGuards {
    #[guard]
    fn first(self, _ctx: Context) {
        Clause::Key(self.keys[0])
    }

    #[guard]
    fn second(self, _ctx: Context) {
        Clause::Key(self.keys[1])
    }

    #[guard(cached)]
    fn third(self) {
        Clause::Key(self.keys[2])
    }

    #[guard(cached)]
    fn fourth(self) {
        Clause::Key(self.keys[3])
    }

    #[then(guarded_by = "[Self::first, Self::second, Self::third, Self::fourth, Self::first]")]
    fn pay(self, ctx: Context) {
        ctx.template()
            .add_output(Amount::from_sat(1_000), &self.keys[0], None)?
            .into()
    }
}

impl Contract for ManyGuards {
    declare! {actions, Self::pay}
}

#[test]
fn more_than_two_guards_require_every_distinct_signer() {
    let contract = ManyGuards {
        keys: [key(1), key(2), key(3), key(4)],
    };
    let compiled = contract.compile(context()).unwrap();
    compiled.validate().unwrap();
    let Some(sapio::contract::abi::object::SupportedDescriptors::XOnly(Descriptor::Tr(tree))) =
        compiled.descriptor
    else {
        panic!("expected a Taproot descriptor");
    };
    let leaves: Vec<_> = tree.leaves().collect();
    assert_eq!(leaves.len(), 1);
    let policy = leaves[0].miniscript().lift().unwrap();
    assert_eq!(policy.minimum_n_keys(), Some(4));
    for key in contract.keys {
        assert!(policy
            .clone()
            .entails(Clause::Key(key).lift().unwrap())
            .unwrap());
    }
}
