use bitcoin::{Amount, Network};
use sapio::contract::{empty, Compilable, Compiled, Contract, DynamicContract};
use sapio::{continuation, declare, guard, Context};
use sapio_base::covenant::LoweringPlan;
use sapio_base::{effects::MapEffectDB, Clause};
use std::cell::RefCell;
use std::sync::Arc;

#[derive(Default)]
struct Updates {
    calls: RefCell<Vec<String>>,
}
impl Updates {
    #[guard]
    fn signed(self, _ctx: Context) {
        Clause::Key(
            bitcoin::XOnlyPublicKey::from_slice(&[
                0x79, 0xbe, 0x66, 0x7e, 0xf9, 0xdc, 0xbb, 0xac, 0x55, 0xa0, 0x62, 0x95, 0xce, 0x87,
                0x0b, 0x07, 0x02, 0x9b, 0xfc, 0xdb, 0x2d, 0xce, 0x28, 0xd9, 0x59, 0xf2, 0x81, 0x5b,
                0x16, 0xf8, 0x17, 0x98,
            ])
            .unwrap(),
        )
    }
    #[continuation(guarded_by = "[Self::signed]", coerce_args = "Ok", web_api)]
    fn update(self, ctx: Context, _args: Option<u64>) {
        self.calls
            .borrow_mut()
            .push(String::from(ctx.path().as_ref().clone()));
        empty()
    }
}
impl Contract for Updates {
    declare! {updatable<Option<u64>>, Self::update}
}

fn context(name: &str) -> Context {
    let effects: MapEffectDB = serde_json::from_value(serde_json::json!({
        "effects": {"updates/@action/update/@suggested": {name: 7}}
    }))
    .unwrap();
    Context::new(
        Network::Regtest,
        Amount::ZERO,
        LoweringPlan::Native,
        "updates".try_into().unwrap(),
        Arc::new(effects),
        None,
    )
}

#[test]
fn malformed_effect_names_fail_before_continuation_callbacks() {
    for name in ["a/b", "@guard", "#7", "not a name"] {
        let contract = Updates::default();
        assert!(contract.compile(context(name)).is_err(), "{name}");
        assert!(contract.calls.borrow().is_empty(), "{name}");
    }
}

#[test]
fn valid_effect_and_continuation_paths_survive_artifact_round_trips() {
    let contract = Updates::default();
    let compiled = contract.compile(context("request_1")).unwrap();
    assert_eq!(
        *contract.calls.borrow(),
        [
            "updates/@action/update/@suggested/@default_effect",
            "updates/@action/update/@suggested/@effects/request_1",
        ]
    );
    let restored: Compiled =
        serde_json::from_str(&serde_json::to_string(&compiled).unwrap()).unwrap();
    assert_eq!(restored, compiled);
    assert_eq!(restored.continue_apis.len(), 1);
    assert_eq!(
        String::from(
            restored
                .continue_apis
                .values()
                .next()
                .unwrap()
                .path
                .as_ref()
                .clone()
        ),
        "updates/@action/update/@suggested"
    );
}

fn bad_name() -> Option<Box<dyn sapio::contract::actions::CallableAsFoF<Updates, Option<u64>>>> {
    let mut action = Updates::update()?;
    action.rename(Arc::new("bad/name".into()));
    Some(action)
}

#[test]
fn invalid_dynamic_action_names_are_rejected_before_execution() {
    let contract = DynamicContract {
        then: vec![],
        finish: vec![],
        finish_or: vec![bad_name],
        data: Updates::default(),
        metadata_f: Box::new(|_, _| Ok(Default::default())),
        ensure_amount_f: Box::new(|_, _| Ok(Amount::ZERO)),
    };
    assert!(contract.compile(context("request_1")).is_err());
    assert!(contract.data.calls.borrow().is_empty());
}
