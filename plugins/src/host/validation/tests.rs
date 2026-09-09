use super::CallSchema;
use crate::host::plugin_handle::SyncModuleLocator;
use crate::host::{PluginHandle, WasmPluginHandle};
use crate::{ContextualArguments, CreateArgs};
use bitcoin::hashes::sha256;
use bitcoin::util::psbt::PartiallySignedTransaction;
use sapio::contract::CompilationError;
use sapio_base::effects::EffectPath;
use sapio_ctv_emulator_trait::{CTVEmulator, Clause, EmulatorError};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct FixtureCache(PathBuf);

impl FixtureCache {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "sapio-call-schema-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn load<O>(&self, source: &str, counter: &Arc<CreateCounter>) -> WasmPluginHandle<O> {
        let emulator: Arc<dyn CTVEmulator> = counter.clone();
        WasmPluginHandle::new(
            self.0.clone(),
            &emulator,
            SyncModuleLocator::Bytes(wasmer::wat2wasm(source.as_bytes()).unwrap().into_owned()),
            bitcoin::Network::Regtest,
            None,
        )
        .unwrap()
    }
}

impl Drop for FixtureCache {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[derive(Default)]
struct CreateCounter(AtomicUsize);

impl CreateCounter {
    fn calls(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }
}

impl CTVEmulator for CreateCounter {
    fn get_signer_for(&self, hash: sha256::Hash) -> Result<Clause, EmulatorError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(Clause::TxTemplate(hash))
    }

    fn sign(
        &self,
        psbt: PartiallySignedTransaction,
    ) -> Result<PartiallySignedTransaction, EmulatorError> {
        Ok(psbt)
    }
}

fn escaped(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("\\{byte:02x}")).collect()
}

fn module(api: &Value, response: &Value, extra: &str, name: &str) -> String {
    let api = escaped(&serde_json::to_vec(api).unwrap());
    let response = escaped(&serde_json::to_vec(response).unwrap());
    format!(
        r#"(module
      (import "env" "sapio_v1_wasm_plugin_ctv_emulator_signer_for"
        (func $mark_create (param i32) (result i32)))
      (import "env" "sapio_v1_wasm_plugin_create_contract"
        (func $nested_create (param i32 i32 i32 i32 i32) (result i32)))
      (memory (export "memory") 1)
      (data (i32.const 8) "ready\00")
      (data (i32.const 1024) "{api}\00")
      (data (i32.const 4096) "{response}\00")
      (func (export "sapio_v1_wasm_plugin_client_allocate_bytes")
        (param i32) (result i32) i32.const 16384)
      (func (export "sapio_v1_wasm_plugin_client_drop_allocation") (param i32))
      (func (export "sapio_v1_wasm_plugin_client_get_create_arguments")
        (result i32) i32.const 1024)
      (func (export "sapio_v1_wasm_plugin_client_get_name") (result i32) {name})
      (func (export "sapio_v1_wasm_plugin_client_get_logo") (result i32) i32.const 8)
      (func (export "sapio_v1_wasm_plugin_client_create") (param i32 i32) (result i32)
        i32.const 128 call $mark_create drop i32.const 4096)
      (func (export "sapio_v1_wasm_plugin_entry_point"))
      {extra}
    )"#
    )
}

fn api(arguments: Value, returns: Value) -> Value {
    json!({
        "arguments": {
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "required": ["arguments", "context"],
            "properties": {
                "arguments": arguments,
                "context": {
                    "type": "object",
                    "required": ["amount", "network"],
                    "properties": {
                        "amount": {"type": "integer", "minimum": 1000},
                        "network": {"const": "Regtest"}
                    }
                }
            }
        },
        "returns": returns
    })
}

fn quantity_schema() -> Value {
    json!({
        "type": "object", "required": ["quantity"],
        "properties": {"quantity": {"type": "integer", "minimum": 1}},
        "additionalProperties": false
    })
}

fn request(arguments: Value) -> CreateArgs<Value> {
    CreateArgs {
        arguments,
        context: ContextualArguments {
            network: bitcoin::Network::Regtest,
            amount: bitcoin::Amount::from_sat(1000),
            effects: Default::default(),
            ordinals_info: None,
        },
    }
}

fn path() -> EffectPath {
    EffectPath::try_from("schema_check").unwrap()
}

fn assert_schema_error(error: CompilationError, side: &str) {
    match error {
        CompilationError::ModuleFailedAPICheck(message) => {
            assert!(message.starts_with(side), "{message}");
        }
        other => panic!("expected {side} schema rejection, got {other}"),
    }
}

fn output_schema(schema: Value) -> Result<CallSchema, CompilationError> {
    CallSchema::from_json(
        &serde_json::to_vec(&json!({"arguments": {}, "returns": schema})).unwrap(),
    )
}

#[test]
fn integer_comparisons_preserve_precision_above_two_to_the_53() {
    let boundary = 9_007_199_254_740_993u64;
    for (keyword, valid, invalid) in [
        ("minimum", boundary, boundary - 1),
        ("maximum", boundary, boundary + 1),
        ("exclusiveMinimum", boundary + 1, boundary),
        ("exclusiveMaximum", boundary - 1, boundary),
    ] {
        let mut schema = json!({"type": "integer"});
        schema[keyword] = json!(boundary);
        let compiled = output_schema(schema).unwrap();
        assert!(
            compiled.validate_output(&json!(valid)).is_ok(),
            "{keyword}: {valid}"
        );
        assert_schema_error(
            compiled.validate_output(&json!(invalid)).unwrap_err(),
            "Output",
        );
    }
    let compiled = output_schema(json!({"type": "integer", "minimum": u64::MAX})).unwrap();
    assert!(compiled.validate_output(&json!(u64::MAX)).is_ok());
    assert_schema_error(
        compiled.validate_output(&json!(u64::MAX - 1)).unwrap_err(),
        "Output",
    );
}

#[test]
fn draft_seven_conditionals_validate_the_selected_branch() {
    let compiled = output_schema(json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object", "required": ["kind", "value"],
        "properties": {"kind": {"enum": ["count", "name"]}},
        "if": {"properties": {"kind": {"const": "count"}}},
        "then": {"properties": {"value": {"type": "integer", "minimum": 1}}},
        "else": {"properties": {"value": {"type": "string", "minLength": 1}}}
    }))
    .unwrap();
    for valid in [
        json!({"kind": "count", "value": 1}),
        json!({"kind": "name", "value": "alice"}),
    ] {
        assert!(compiled.validate_output(&valid).is_ok());
    }
    for invalid in [
        json!({"kind": "count", "value": "alice"}),
        json!({"kind": "name", "value": 1}),
    ] {
        assert_schema_error(compiled.validate_output(&invalid).unwrap_err(), "Output");
    }
}

#[test]
fn malformed_patterns_unsupported_drafts_and_unresolved_references_are_rejected() {
    let cache = FixtureCache::new();
    let local = cache.0.join("external-schema.json");
    std::fs::write(&local, br#"{"type":"integer"}"#).unwrap();
    for schema in [
        json!({"type": "string", "pattern": "("}),
        json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "integer"}),
        json!({"$ref": "#/definitions/missing"}),
        json!({"$ref": "https://example.invalid/external-schema.json"}),
        json!({"$ref": format!("file://{}", local.display())}),
    ] {
        let error = output_schema(schema.clone())
            .err()
            .expect("invalid or nonlocal schema must fail");
        assert!(
            matches!(error, CompilationError::ModuleFailedAPICheck(_)),
            "{schema}: {error}"
        );
    }
}

#[test]
fn recursive_local_references_validate_finite_objects() {
    let compiled = output_schema(json!({
        "$ref": "#/definitions/node",
        "definitions": {"node": {
            "type": "object", "required": ["value", "next"],
            "additionalProperties": false,
            "properties": {
                "value": {"type": "integer"},
                "next": {"anyOf": [{"type": "null"}, {"$ref": "#/definitions/node"}]}
            }
        }}
    }))
    .unwrap();
    let valid = json!({"value": 1, "next": {"value": 2, "next": null}});
    assert!(compiled.validate_output(&valid).is_ok());
    let invalid = json!({"value": 1, "next": {"value": "bad", "next": null}});
    assert_schema_error(compiled.validate_output(&invalid).unwrap_err(), "Output");
}

#[test]
fn actual_arguments_and_context_are_checked_before_guest_create() {
    let cache = FixtureCache::new();
    let counter = Arc::new(CreateCounter::default());
    let source = module(
        &api(quantity_schema(), json!({"type": "integer"})),
        &json!({"Ok": 7}),
        "",
        "i32.const 8",
    );
    let mut plugin = cache.load::<Value>(&source, &counter);
    let mut low_amount = request(json!({"quantity": 1}));
    low_amount.context.amount = bitcoin::Amount::from_sat(999);
    let mut wrong_network = request(json!({"quantity": 1}));
    wrong_network.context.network = bitcoin::Network::Signet;
    for invalid in [
        request(json!({"quantity": "one"})),
        low_amount,
        wrong_network,
    ] {
        assert_schema_error(plugin.call(&path(), &invalid).unwrap_err(), "Input");
        assert_eq!(counter.calls(), 0, "invalid input must not invoke create");
    }
    assert_eq!(
        plugin
            .call(&path(), &request(json!({"quantity": 1})))
            .unwrap(),
        json!(7)
    );
    assert_eq!(counter.calls(), 1);
}

#[test]
fn actual_calls_preserve_advertised_large_integer_bounds() {
    let cache = FixtureCache::new();
    let counter = Arc::new(CreateCounter::default());
    let boundary = 9_007_199_254_740_993u64;
    let mut arguments = quantity_schema();
    arguments["properties"]["quantity"]["minimum"] = json!(boundary);
    let api = api(arguments, json!({"type": "integer", "minimum": boundary}));
    let source = module(&api, &json!({"Ok": boundary}), "", "i32.const 8");
    let mut plugin = cache.load::<Value>(&source, &counter);
    assert_schema_error(
        plugin
            .call(&path(), &request(json!({"quantity": boundary - 1})))
            .unwrap_err(),
        "Input",
    );
    assert_eq!(counter.calls(), 0);
    assert_eq!(
        plugin
            .call(&path(), &request(json!({"quantity": boundary})))
            .unwrap(),
        json!(boundary),
    );
    assert_eq!(counter.calls(), 1);

    let source = module(&api, &json!({"Ok": boundary - 1}), "", "i32.const 8");
    let mut plugin = cache.load::<Value>(&source, &counter);
    assert_schema_error(
        plugin
            .call(&path(), &request(json!({"quantity": boundary})))
            .unwrap_err(),
        "Output",
    );
    assert_eq!(counter.calls(), 2);
}

#[test]
fn successful_output_is_checked_before_the_call_returns() {
    let cache = FixtureCache::new();
    let counter = Arc::new(CreateCounter::default());
    let source = module(
        &api(quantity_schema(), json!({"type": "integer"})),
        &json!({"Ok": "wrong output"}),
        "",
        "i32.const 8",
    );
    let mut plugin = cache.load::<Value>(&source, &counter);
    assert_schema_error(
        plugin
            .call(&path(), &request(json!({"quantity": 1})))
            .unwrap_err(),
        "Output",
    );
    assert_eq!(counter.calls(), 1, "output rejection follows guest create");
}

#[test]
fn ordinary_module_errors_are_preserved() {
    let cache = FixtureCache::new();
    let counter = Arc::new(CreateCounter::default());
    let source = module(
        &api(quantity_schema(), json!({"type": "integer"})),
        &json!({"Err": "business rule declined"}),
        "",
        "i32.const 8",
    );
    let mut plugin = cache.load::<Value>(&source, &counter);
    match plugin
        .call(&path(), &request(json!({"quantity": 1})))
        .unwrap_err()
    {
        CompilationError::ModuleCompilationErrorUnsendable(message) => {
            assert_eq!(message, "business rule declined")
        }
        other => panic!("expected the ordinary module error, got {other}"),
    }
    assert_eq!(counter.calls(), 1);
}

#[test]
fn schema_acceptance_still_requires_the_callers_result_type() {
    let cache = FixtureCache::new();
    let counter = Arc::new(CreateCounter::default());
    let source = module(
        &api(quantity_schema(), json!({"type": "integer"})),
        &json!({"Ok": 7}),
        "",
        "i32.const 8",
    );
    let mut plugin = cache.load::<String>(&source, &counter);
    assert!(matches!(
        plugin.call(&path(), &request(json!({"quantity": 1}))),
        Err(CompilationError::DeserializationError(_))
    ));
    assert_eq!(counter.calls(), 1);
}

#[test]
fn receiver_can_accept_a_shared_version_and_additional_variants() {
    let cache = FixtureCache::new();
    let counter = Arc::new(CreateCounter::default());
    let arguments = json!({"oneOf": [
        {"type": "object", "required": ["BatchingTraitVersion0_1_1"], "additionalProperties": false,
         "properties": {"BatchingTraitVersion0_1_1": {"type": "object", "required": ["payments", "feerate_per_byte"],
             "properties": {"payments": {"type": "array"}, "feerate_per_byte": {"type": "integer"}}}}},
        {"type": "object", "required": ["Direct"], "additionalProperties": false,
         "properties": {"Direct": {"type": "integer"}}}
    ]});
    let source = module(
        &api(arguments, json!({"type": "integer"})),
        &json!({"Ok": 7}),
        "",
        "i32.const 8",
    );
    let mut plugin = cache.load::<Value>(&source, &counter);
    for arguments in [
        json!({"BatchingTraitVersion0_1_1": {"payments": [], "feerate_per_byte": 0}}),
        json!({"Direct": 3}),
    ] {
        assert_eq!(plugin.call(&path(), &request(arguments)).unwrap(), json!(7));
    }
    assert_eq!(counter.calls(), 2);
}

#[test]
fn raw_nested_create_import_enforces_the_childs_input_and_output_schemas() {
    let cache = FixtureCache::new();
    for (arguments, output, error_side, expected_calls) in [
        (json!({"quantity": "bad"}), json!(7), Some("Input"), 0),
        (
            json!({"quantity": 1}),
            json!("wrong output"),
            Some("Output"),
            1,
        ),
        (json!({"quantity": 1}), json!(7), None, 1),
    ] {
        let counter = Arc::new(CreateCounter::default());
        let child = module(
            &api(quantity_schema(), json!({"type": "integer"})),
            &json!({"Ok": output}),
            "",
            "i32.const 8",
        );
        let key = cache.load::<Value>(&child, &counter).id();
        let key = escaped(&hex::decode(key.to_string()).unwrap());
        let path = serde_json::to_vec(&path()).unwrap();
        let arguments = serde_json::to_vec(&request(arguments)).unwrap();
        let extra = format!(
            r#"
            (data (i32.const 64) "{key}")
            (data (i32.const 8192) "{}")
            (data (i32.const 12288) "{}")"#,
            escaped(&path),
            escaped(&arguments)
        );
        let call = format!("i32.const 8192 i32.const {} i32.const 64 i32.const 12288 i32.const {} call $nested_create", path.len(), arguments.len());
        // get_name invokes the raw import directly, without constructing a
        // typed client handle or calling the parent's create/schema gate.
        let parent = module(
            &api(json!({}), json!({})),
            &json!({"Ok": null}),
            &extra,
            &call,
        );
        let mut parent = cache.load::<Value>(&parent, &counter);
        let result: Result<Value, String> =
            serde_json::from_str(&parent.get_name().unwrap()).unwrap();
        match error_side {
            Some(side) => assert!(result
                .unwrap_err()
                .contains(&format!("{side} JSON does not satisfy"))),
            None => assert_eq!(result.unwrap(), json!(7)),
        }
        assert_eq!(counter.calls(), expected_calls);
    }
}
