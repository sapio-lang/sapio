use super::CallSchema;
use crate::host::plugin_handle::SyncModuleLocator;
use crate::host::{PluginHandle, WasmPluginHandle};
use crate::{ContextualArguments, CreateArgs};
use sapio::contract::CompilationError;
use sapio_base::effects::EffectPath;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

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

    fn load<O>(&self, source: &str) -> WasmPluginHandle<O> {
        WasmPluginHandle::new(
            self.0.clone(),
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

fn calls<O: for<'a> serde::Deserialize<'a>>(plugin: &mut WasmPluginHandle<O>) -> usize {
    plugin.get_logo().unwrap().parse().unwrap()
}

fn escaped(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("\\{byte:02x}")).collect()
}

fn module(api: &Value, response: &Value, extra: &str, name: &str) -> String {
    let api = escaped(&serde_json::to_vec(api).unwrap());
    let response = escaped(&serde_json::to_vec(response).unwrap());
    format!(
        r#"(module
      (import "env" "sapio_v1_wasm_plugin_create_contract"
        (func $nested_create (param i32 i32 i32 i32 i32) (result i32)))
      (global $calls (mut i32) (i32.const 0))
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
      (func (export "sapio_v1_wasm_plugin_client_get_logo") (result i32)
        i32.const 512 global.get $calls i32.const 48 i32.add i32.store8 i32.const 512)
      (func (export "sapio_v1_wasm_plugin_client_create") (param i32 i32) (result i32)
        global.get $calls i32.const 1 i32.add global.set $calls i32.const 4096)
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
                    "required": ["amount", "network", "lowering"],
                    "properties": {
                        "amount": {"type": "integer", "minimum": 1000},
                        "network": {"const": "Regtest"},
                        "lowering": {"const": "Native"}
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
            lowering: sapio_base::covenant::LoweringPlan::Native,
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
    let source = module(
        &api(quantity_schema(), json!({"type": "integer"})),
        &json!({"Ok": 7}),
        "",
        "i32.const 8",
    );
    let mut plugin = cache.load::<Value>(&source);
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
        assert_eq!(
            calls(&mut plugin),
            0,
            "invalid input must not invoke create"
        );
    }
    assert_eq!(
        plugin
            .call(&path(), &request(json!({"quantity": 1})))
            .unwrap(),
        json!(7)
    );
    assert_eq!(calls(&mut plugin), 1);
}

#[test]
fn permissive_module_schemas_cannot_accept_invalid_lowering_plans() {
    use sapio_base::covenant::{CovenantError, LoweringPlan};

    let cache = FixtureCache::new();
    let source = module(
        &json!({"arguments": {}, "returns": {}}),
        &json!({"Ok": 7}),
        "",
        "i32.const 8",
    );
    let mut plugin = cache.load::<Value>(&source);
    let key = "tpubD6NzVbkrYhZ4Wf398td3H8YhWBsXx9Sxa4W3cQWkNW3N3DHSNB2qtPoUMXrA6JNaPxodQfRpoZNE5tGM9iZ4xfUEFRJEJvfs8W5paUagYCE"
        .parse().unwrap();
    for lowering in [
        LoweringPlan::CtvEmulation {
            signers: vec![],
            threshold: 1,
        },
        LoweringPlan::CtvEmulation {
            signers: vec![key, key],
            threshold: 1,
        },
    ] {
        let mut input = request(json!({}));
        input.context.lowering = lowering;
        assert!(matches!(
            plugin.call(&path(), &input),
            Err(CompilationError::Covenant(
                CovenantError::InvalidThreshold { .. } | CovenantError::DuplicateSigner { .. }
            ))
        ));
        assert_eq!(
            calls(&mut plugin),
            0,
            "invalid public inputs must not invoke create"
        );
    }
    assert_eq!(plugin.call(&path(), &request(json!({}))).unwrap(), json!(7));
    assert_eq!(calls(&mut plugin), 1);
}

#[test]
fn raw_nested_calls_validate_lowering_before_loading_the_child() {
    use sapio_base::covenant::LoweringPlan;

    let cache = FixtureCache::new();
    let mut input = request(json!({}));
    input.context.lowering = LoweringPlan::CtvEmulation {
        signers: vec![],
        threshold: 1,
    };
    let path = serde_json::to_vec(&path()).unwrap();
    let arguments = serde_json::to_vec(&input).unwrap();
    let extra = format!(
        r#"(data (i32.const 8192) "{}") (data (i32.const 12288) "{}")"#,
        escaped(&path),
        escaped(&arguments),
    );
    let call = format!(
        "i32.const 8192 i32.const {} i32.const 64 i32.const 12288 i32.const {} call $nested_create",
        path.len(),
        arguments.len()
    );
    // The zero key is absent from the cache. Lowering must fail before even
    // trying to load it, and this metadata callback bypasses the typed call API.
    let source = module(
        &json!({"arguments": {}, "returns": {}}),
        &json!({"Ok": 7}),
        &extra,
        &call,
    );
    let error = cache.load::<Value>(&source).get_name().unwrap_err();
    assert!(
        error.to_string().contains("CTV signer threshold"),
        "{error}"
    );
}

#[test]
fn actual_calls_preserve_advertised_large_integer_bounds() {
    let cache = FixtureCache::new();
    let boundary = 9_007_199_254_740_993u64;
    let mut arguments = quantity_schema();
    arguments["properties"]["quantity"]["minimum"] = json!(boundary);
    let api = api(arguments, json!({"type": "integer", "minimum": boundary}));
    let source = module(&api, &json!({"Ok": boundary}), "", "i32.const 8");
    let mut plugin = cache.load::<Value>(&source);
    assert_schema_error(
        plugin
            .call(&path(), &request(json!({"quantity": boundary - 1})))
            .unwrap_err(),
        "Input",
    );
    assert_eq!(calls(&mut plugin), 0);
    assert_eq!(
        plugin
            .call(&path(), &request(json!({"quantity": boundary})))
            .unwrap(),
        json!(boundary),
    );
    assert_eq!(calls(&mut plugin), 1);

    let source = module(&api, &json!({"Ok": boundary - 1}), "", "i32.const 8");
    let mut plugin = cache.load::<Value>(&source);
    assert_schema_error(
        plugin
            .call(&path(), &request(json!({"quantity": boundary})))
            .unwrap_err(),
        "Output",
    );
    assert_eq!(calls(&mut plugin), 1);
}

#[test]
fn successful_output_is_checked_before_the_call_returns() {
    let cache = FixtureCache::new();
    let source = module(
        &api(quantity_schema(), json!({"type": "integer"})),
        &json!({"Ok": "wrong output"}),
        "",
        "i32.const 8",
    );
    let mut plugin = cache.load::<Value>(&source);
    assert_schema_error(
        plugin
            .call(&path(), &request(json!({"quantity": 1})))
            .unwrap_err(),
        "Output",
    );
    assert_eq!(
        calls(&mut plugin),
        1,
        "output rejection follows guest create"
    );
}

#[test]
fn ordinary_module_errors_are_preserved() {
    let cache = FixtureCache::new();
    let source = module(
        &api(quantity_schema(), json!({"type": "integer"})),
        &json!({"Err": "business rule declined"}),
        "",
        "i32.const 8",
    );
    let mut plugin = cache.load::<Value>(&source);
    match plugin
        .call(&path(), &request(json!({"quantity": 1})))
        .unwrap_err()
    {
        CompilationError::ModuleCompilationErrorUnsendable(message) => {
            assert_eq!(message, "business rule declined")
        }
        other => panic!("expected the ordinary module error, got {other}"),
    }
    assert_eq!(calls(&mut plugin), 1);
}

#[test]
fn schema_acceptance_still_requires_the_callers_result_type() {
    let cache = FixtureCache::new();
    let source = module(
        &api(quantity_schema(), json!({"type": "integer"})),
        &json!({"Ok": 7}),
        "",
        "i32.const 8",
    );
    let mut plugin = cache.load::<String>(&source);
    assert!(matches!(
        plugin.call(&path(), &request(json!({"quantity": 1}))),
        Err(CompilationError::DeserializationError(_))
    ));
    assert_eq!(calls(&mut plugin), 1);
}

#[test]
fn receiver_can_accept_a_shared_version_and_additional_variants() {
    let cache = FixtureCache::new();
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
    let mut plugin = cache.load::<Value>(&source);
    for arguments in [
        json!({"BatchingTraitVersion0_1_1": {"payments": [], "feerate_per_byte": 0}}),
        json!({"Direct": 3}),
    ] {
        assert_eq!(plugin.call(&path(), &request(arguments)).unwrap(), json!(7));
    }
    assert_eq!(calls(&mut plugin), 2);
}

#[test]
fn raw_nested_create_import_enforces_the_childs_input_and_output_schemas() {
    let cache = FixtureCache::new();
    for (arguments, output, error_side) in [
        (json!({"quantity": "bad"}), json!(7), Some("Input")),
        (
            json!({"quantity": 1}),
            json!("wrong output"),
            Some("Output"),
        ),
        (json!({"quantity": 1}), json!(7), None),
    ] {
        let child = module(
            &api(quantity_schema(), json!({"type": "integer"})),
            &json!({"Ok": output}),
            "",
            "i32.const 8",
        );
        let child = if error_side == Some("Input") {
            // A bad argument must be rejected before this adversarial create runs.
            child.replace(
                "global.get $calls i32.const 1 i32.add global.set $calls i32.const 4096",
                "unreachable",
            )
        } else {
            child
        };
        let key = cache.load::<Value>(&child).id();
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
        let mut parent = cache.load::<Value>(&parent);
        let result: Result<Value, String> =
            serde_json::from_str(&parent.get_name().unwrap()).unwrap();
        match error_side {
            Some(side) => assert!(result
                .unwrap_err()
                .contains(&format!("{side} JSON does not satisfy"))),
            None => assert_eq!(result.unwrap(), json!(7)),
        }
    }
}
