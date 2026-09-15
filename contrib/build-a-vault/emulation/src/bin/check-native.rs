//! Compare generated WASM custody artifacts with native execution of every block.
//!
//! Run with one argument: the generated directory containing manifest.json.

use build_a_vault_blocks::{
    BlockDelay, DelayedWallet, Destination, FixedVault, Quorum, Recovery, Release, Signer,
};
use build_a_vault_emulation::{EmulationOracle, OpVault};
use sapio::contract::{Compiled, Context};
use sapio_base::effects::{EffectPath, PathFragment};
use sapio_base::plugin_args::ContextualArguments;
use sapio_base::serialization_helpers::SArc;
use sapio_wasm_plugin::client::plugin::Callable;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

type CheckResult<T> = Result<T, Box<dyn Error>>;

#[derive(Deserialize)]
struct Manifest {
    modules: Vec<Module>,
    recipes: Vec<Recipe>,
}

#[derive(Deserialize)]
struct Module {
    name: String,
    key: String,
}

#[derive(Deserialize)]
struct Recipe {
    name: String,
    patch: PathBuf,
    artifact: PathBuf,
}

#[derive(Deserialize)]
struct Patch {
    version: u32,
    nodes: Vec<Node>,
    connections: Vec<Connection>,
    context: ContextualArguments,
}

#[derive(Deserialize)]
struct Node {
    id: String,
    #[serde(rename = "moduleKey")]
    module_key: String,
    arguments: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum ValueConnection {
    Value,
}

#[derive(Deserialize)]
struct Connection {
    #[serde(rename = "kind")]
    _kind: ValueConnection,
    source: String,
    #[serde(rename = "sourcePath")]
    source_path: String,
    target: String,
    #[serde(rename = "targetPath")]
    target_path: String,
}

fn read<T: DeserializeOwned>(path: &Path) -> CheckResult<T> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))?)
}

fn context(arguments: &ContextualArguments, module_key: &str) -> CheckResult<Context> {
    arguments.lowering.validate()?;
    // Match CLI's root call and Plugin::create_result's two appended fragments.
    // Do not read this path from the expected artifact: it is an independent
    // input to deterministic key selection and contract compilation.
    let caller = EffectPath::push(None, PathFragment::Root);
    let plugin = EffectPath::push(Some(caller), PathFragment::Root);
    let path = EffectPath::push_owned(
        Some(plugin),
        PathFragment::Named(SArc(Arc::new(module_key.to_owned()))),
    );
    Ok(Context::new(
        arguments.network,
        arguments.amount,
        arguments.lowering.clone(),
        path,
        Arc::new(arguments.effects.clone()),
        arguments.ordinals_info.clone(),
    ))
}

fn call<T: DeserializeOwned + Callable>(arguments: Value, ctx: Context) -> CheckResult<Value>
where
    T::Output: Serialize,
{
    Ok(serde_json::to_value(T::deserialize(arguments)?.call(ctx)?)?)
}

fn invoke(name: &str, arguments: Value, ctx: Context) -> CheckResult<Value> {
    match name {
        "signer" => call::<Signer>(arguments, ctx),
        "quorum" => call::<Quorum>(arguments, ctx),
        "block-delay" => call::<BlockDelay>(arguments, ctx),
        "destination" => call::<Destination>(arguments, ctx),
        "recovery" => call::<Recovery>(arguments, ctx),
        "release" => call::<Release>(arguments, ctx),
        "fixed-vault" => call::<FixedVault>(arguments, ctx),
        "delayed-wallet" => call::<DelayedWallet>(arguments, ctx),
        "emulation-oracle" => call::<EmulationOracle>(arguments, ctx),
        "op-vault" => call::<OpVault>(arguments, ctx),
        _ => Err(format!("unknown building block {name}").into()),
    }
}

fn connect(arguments: &mut Value, pointer: &str, value: Value) -> CheckResult<()> {
    let Some(pointer) = pointer.strip_prefix('/') else {
        return Err("a recipe connection needs an argument field".into());
    };
    let mut fields = pointer.split('/').peekable();
    let mut cursor = arguments;
    while let Some(field) = fields.next() {
        let field = field.replace("~1", "/").replace("~0", "~");
        let object = cursor
            .as_object_mut()
            .ok_or("recipe connection traverses a non-object argument")?;
        if fields.peek().is_none() {
            object.insert(field, value);
            return Ok(());
        }
        cursor = object
            .entry(field)
            .or_insert_with(|| Value::Object(Default::default()));
    }
    Err("recipe connection has no argument field".into())
}

fn main() -> CheckResult<()> {
    let mut arguments = std::env::args_os().skip(1);
    let directory = PathBuf::from(
        arguments
            .next()
            .ok_or("usage: check-native GENERATED_DIR")?,
    );
    if arguments.next().is_some() {
        return Err("usage: check-native GENERATED_DIR".into());
    }
    let manifest: Manifest = read(&directory.join("manifest.json"))?;
    let modules: BTreeMap<_, _> = manifest
        .modules
        .iter()
        .map(|module| (module.key.as_str(), module.name.as_str()))
        .collect();
    if manifest.modules.len() != 10 || modules.len() != 10 || manifest.recipes.is_empty() {
        return Err("manifest must contain ten distinct modules and generated recipes".into());
    }
    let mut exercised = BTreeSet::new();
    for recipe in &manifest.recipes {
        let patch: Patch = read(&directory.join(&recipe.patch))?;
        if patch.version != 1 || patch.nodes.is_empty() {
            return Err(format!("{}: expected a nonempty version-one patch", recipe.name).into());
        }
        let mut values: BTreeMap<&str, Value> = BTreeMap::new();
        // build.py emits recipes in dependency order; fail explicitly if a
        // saved recipe no longer meets that convention.
        for node in &patch.nodes {
            let name = modules
                .get(node.module_key.as_str())
                .ok_or("patch refers to a module absent from the manifest")?;
            let mut arguments = node.arguments.clone();
            for connection in patch
                .connections
                .iter()
                .filter(|wire| wire.target == node.id)
            {
                let source = values
                    .get(connection.source.as_str())
                    .ok_or("recipe nodes are not in dependency order")?;
                let value = source
                    .pointer(&connection.source_path)
                    .ok_or("recipe source value has no advertised result field")?;
                connect(&mut arguments, &connection.target_path, value.clone())?;
            }
            let result = invoke(name, arguments, context(&patch.context, &node.module_key)?)
                .map_err(|error| format!("{} / {}: {error}", recipe.name, node.id))?;
            if values.insert(&node.id, result).is_some() {
                return Err("recipe contains duplicate node identifiers".into());
            }
            exercised.insert(*name);
        }
        let terminal = patch.nodes.last().ok_or("recipe has no terminal block")?;
        let actual = values
            .get(terminal.id.as_str())
            .ok_or("terminal did not run")?;
        let compiled: Compiled = serde_json::from_value(actual.clone())?;
        compiled.validate()?;
        let expected: Value = read(&directory.join(&recipe.artifact))?;
        if actual != &expected {
            return Err(format!(
                "{}: native contract differs from generated WASM artifact",
                recipe.name
            )
            .into());
        }
        println!(
            "{}: {} native block calls reproduce the full WASM artifact",
            recipe.name,
            patch.nodes.len()
        );
    }
    for module in &manifest.modules {
        if !exercised.contains(module.name.as_str()) {
            return Err(format!("{}: no recipe exercised this native block", module.name).into());
        }
    }
    println!(
        "All ten building blocks have native/WASM parity across {} recipes.",
        manifest.recipes.len()
    );
    Ok(())
}
