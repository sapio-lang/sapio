//! Run one real WASM catalog fixture. The Python driver bounds each process.
use bitcoin::Network;
use sapio::contract::Compiled;
use sapio_base::effects::EffectPath;
use sapio_ctv_emulator_trait::{CTVAvailable, CTVEmulator};
use sapio_wasm_plugin::host::plugin_handle::{SyncModuleLocator, WasmPluginHandle};
use sapio_wasm_plugin::plugin_handle::PluginHandle;
use sapio_wasm_plugin::CreateArgs;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::error::Error;
use std::path::Path;
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    wasm: String,
    input: String,
    dependencies: Vec<String>,
    expect: Expected,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    raw_taproot: Option<bool>,
    clause: Option<String>,
    ctv_count: Option<usize>,
    suggested_count: Option<usize>,
    continuation_count: Option<usize>,
    root_output_values: Option<Vec<u64>>,
    root_lock_time: Option<u32>,
    root_lock_times: Option<Vec<u32>>,
    root_output_values_by_lock_time: Option<BTreeMap<u32, Vec<u64>>>,
    root_required_input_amount: Option<u64>,
}

fn substitute(value: &mut Value, modules: &BTreeMap<String, String>) -> Result<(), Box<dyn Error>> {
    match value {
        Value::String(s) if s.starts_with("MODULE_") => {
            *s = modules
                .get(&s[7..])
                .ok_or("unknown module placeholder")?
                .clone();
        }
        Value::Array(values) => {
            for value in values {
                substitute(value, modules)?;
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                substitute(value, modules)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn check(value: Value, expected: &Expected) -> Result<(), Box<dyn Error>> {
    if let Some(clause) = &expected.clause {
        assert_eq!(value, Value::String(clause.clone()));
        return Ok(());
    }
    let compiled: Compiled = serde_json::from_value(value)?;
    compiled.validate()?;
    if let Some(expected) = expected.raw_taproot {
        assert_eq!(
            matches!(
                compiled.descriptor,
                Some(sapio::contract::object::SupportedDescriptors::Taproot(_))
            ),
            expected
        );
    }
    for (actual, expected) in [
        (compiled.ctv_to_tx.len(), expected.ctv_count),
        (compiled.suggested_txs.len(), expected.suggested_count),
        (compiled.continue_apis.len(), expected.continuation_count),
    ] {
        if let Some(expected) = expected {
            assert_eq!(actual, expected);
        }
    }
    let templates: Vec<_> = compiled
        .ctv_to_tx
        .values()
        .chain(compiled.suggested_txs.values())
        .collect();
    if let Some(values) = &expected.root_output_values {
        assert_eq!(
            templates.len(),
            1,
            "output fixture must identify one transition"
        );
        assert_eq!(
            &templates[0]
                .tx
                .output
                .iter()
                .map(|o| o.value)
                .collect::<Vec<_>>(),
            values
        );
    }
    if let Some(lock_time) = expected.root_lock_time {
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].tx.lock_time, lock_time);
    }
    if let Some(lock_times) = &expected.root_lock_times {
        let mut actual: Vec<_> = templates.iter().map(|t| t.tx.lock_time).collect();
        actual.sort_unstable();
        assert_eq!(&actual, lock_times);
    }
    if let Some(outputs) = &expected.root_output_values_by_lock_time {
        let actual: BTreeMap<_, _> = templates
            .iter()
            .map(|t| {
                (
                    t.tx.lock_time,
                    t.tx.output.iter().map(|o| o.value).collect::<Vec<_>>(),
                )
            })
            .collect();
        assert_eq!(actual.len(), templates.len());
        assert_eq!(&actual, outputs);
    }
    if let Some(required) = expected.root_required_input_amount {
        assert!(!templates.is_empty());
        for template in templates {
            assert_eq!(template.required_input_amount.as_sat(), required);
        }
        assert_eq!(compiled.required_input_amount.as_sat(), required);
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 5 {
        return Err("usage: check_examples WASM_DIR FIXTURE_DIR CASE CACHE_DIR".into());
    }
    let wasm = Path::new(&args[1]);
    let fixtures = Path::new(&args[2]);
    let catalog: BTreeMap<String, Case> =
        serde_json::from_slice(&std::fs::read(fixtures.join("catalog.json"))?)?;
    let case = catalog.get(&args[3]).ok_or("unknown catalog case")?;
    let emulator: Arc<dyn CTVEmulator> = Arc::new(CTVAvailable);
    let load = |case: &Case| -> Result<WasmPluginHandle<Value>, Box<dyn Error>> {
        WasmPluginHandle::new(
            Path::new(&args[4]).to_path_buf(),
            &emulator,
            SyncModuleLocator::Bytes(std::fs::read(wasm.join(&case.wasm))?),
            Network::Regtest,
            None,
        )
    };
    let mut modules = BTreeMap::new();
    let sources = Path::new(&args[4]).join("sources");
    std::fs::create_dir_all(&sources)?;
    for dependency in &case.dependencies {
        let dependency_case = catalog.get(dependency).ok_or("unknown dependency")?;
        let bytes = std::fs::read(wasm.join(&dependency_case.wasm))?;
        let key = wasmer_cache::Hash::generate(&bytes).to_string();
        // Seed trusted fixture sources without a throwaway native compilation.
        // The real nested call still authenticates and compiles these bytes.
        std::fs::write(sources.join(format!("{key}.wasm")), bytes)?;
        modules.insert(dependency.clone(), key);
    }
    let mut input: Value = serde_json::from_slice(&std::fs::read(fixtures.join(&case.input))?)?;
    substitute(&mut input, &modules)?;
    let mut input: CreateArgs<Value> = serde_json::from_value(input)?;
    let path = EffectPath::try_from("example")?;
    let mut plugin = load(case)?;
    assert!(!plugin.get_name()?.trim().is_empty());
    let first = plugin.call(&path, &input)?;
    // Reuse compiled code, with independent memory, fuel and child budgets.
    assert_eq!(
        first,
        plugin.fresh_clone()?.call(&path, &input)?,
        "nondeterministic artifact"
    );
    check(first, &case.expect)?;
    input.arguments = Value::Null;
    let error = plugin
        .fresh_clone()?
        .call(&path, &input)
        .expect_err("null arguments accepted");
    assert!(error.to_string().contains("Input"), "{error}");
    println!(
        "WASM {}: artifact, repeatability and schema rejection passed",
        args[3]
    );
    Ok(())
}
