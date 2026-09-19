//! Authenticated signing context supplied independently of guest evidence.

use bitcoin::hashes::Hash;
use bitcoin::taproot::TapLeafHash;
use wasmer::{
    Function, FunctionEnv, FunctionEnvMut, Global, Imports, Instance, Memory, RuntimeError, Store,
    Value,
};

pub(super) const CONTEXT_NAMESPACE_V2: &str = "sapio_context_v2";
/// Each context read debits the same allowance as guest code and crypto calls.
const TAPLEAF_HASH_FUEL: u64 = 100;

pub(super) struct ContextEnvironment {
    selected_tapleaf: Option<TapLeafHash>,
    bound: Option<BoundContext>,
}

#[derive(Clone)]
struct BoundContext {
    memory: Memory,
    remaining: Global,
    exhausted: Global,
}

fn error(message: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::new(message.to_string())
}

pub(super) fn add_context_imports(
    store: &mut Store,
    imports: &mut Imports,
    selected_tapleaf: Option<TapLeafHash>,
) -> FunctionEnv<ContextEnvironment> {
    let env = FunctionEnv::new(
        store,
        ContextEnvironment {
            selected_tapleaf,
            bound: None,
        },
    );
    imports.define(
        CONTEXT_NAMESPACE_V2,
        "tapleaf_hash",
        Function::new_typed_with_env(store, &env, tapleaf_hash),
    );
    env
}

pub(super) fn bind_context(
    env: &FunctionEnv<ContextEnvironment>,
    store: &mut Store,
    instance: &Instance,
) -> Result<(), RuntimeError> {
    if env.as_ref(store).bound.is_some() {
        return Err(error("WASM context environment is already bound"));
    }
    let bound = BoundContext {
        memory: instance
            .exports
            .get_memory("memory")
            .map_err(error)?
            .clone(),
        remaining: instance
            .exports
            .get_global("wasmer_metering_remaining_points")
            .map_err(error)?
            .clone(),
        exhausted: instance
            .exports
            .get_global("wasmer_metering_points_exhausted")
            .map_err(error)?
            .clone(),
    };
    env.as_mut(store).bound = Some(bound);
    Ok(())
}

fn tapleaf_hash(
    mut env: FunctionEnvMut<'_, ContextEnvironment>,
    output: u32,
) -> Result<i32, RuntimeError> {
    let bound = env
        .data()
        .bound
        .clone()
        .ok_or_else(|| error("WASM context environment is not initialized"))?;
    let Value::I32(exhausted) = bound.exhausted.get(&mut env) else {
        return Err(error("WASM context exhausted global has the wrong type"));
    };
    let Value::I64(remaining) = bound.remaining.get(&mut env) else {
        return Err(error("WASM context remaining global has the wrong type"));
    };
    // Setting remaining points through the middleware API clears exhaustion.
    // Debit its globals directly so all host imports share nonrenewable fuel.
    if exhausted != 0 || (remaining as u64) < TAPLEAF_HASH_FUEL {
        bound.exhausted.set(&mut env, Value::I32(1))?;
        bound.remaining.set(&mut env, Value::I64(0))?;
        return Err(error("WASM context exhausted instance fuel"));
    }
    bound.remaining.set(
        &mut env,
        Value::I64(((remaining as u64) - TAPLEAF_HASH_FUEL) as i64),
    )?;
    let Some(leaf) = env.data().selected_tapleaf else {
        // No selected leaf means no output write, including for a null or
        // otherwise unused pointer. The return value is the option tag.
        return Ok(0);
    };
    let memory = bound.memory.view(&env);
    if u64::from(output) + 32 > memory.data_size() {
        return Err(error("WASM context buffer is outside guest memory"));
    }
    memory
        .write(u64::from(output), leaf.as_byte_array())
        .map_err(error)?;
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sapio_wasm::host::{add_crypto_imports, bind_crypto, store_with_fuel, SHA256_BASE_FUEL};
    use wasmer::Module;
    use wasmer_middlewares::metering::{get_remaining_points, MeteringPoints};

    #[test]
    fn context_reads_and_crypto_debit_one_nonrenewable_budget() {
        for leaf in [None, Some(TapLeafHash::from_byte_array([42; 32]))] {
            let mut store = store_with_fuel(TAPLEAF_HASH_FUEL + SHA256_BASE_FUEL);
            let module = Module::new(
                &store,
                r#"(module
                (import "sapio_context_v2" "tapleaf_hash"
                    (func $leaf (param i32) (result i32)))
                (import "sapio_crypto_v1" "sha256"
                    (func $sha (param i32 i32 i32) (result i32)))
                (memory (export "memory") 1)
                (export "leaf" (func $leaf))
                (export "sha" (func $sha))
                (func))"#,
            )
            .unwrap();
            let mut imports = Imports::new();
            let context = add_context_imports(&mut store, &mut imports, leaf);
            let crypto = add_crypto_imports(&mut store, &mut imports);
            let instance = Instance::new(&mut store, &module, &imports).unwrap();
            bind_context(&context, &mut store, &instance).unwrap();
            bind_crypto(&crypto, &mut store, &instance).unwrap();
            let read = instance
                .exports
                .get_typed_function::<u32, i32>(&store, "leaf")
                .unwrap();
            assert_eq!(read.call(&mut store, 0).unwrap(), i32::from(leaf.is_some()));
            assert_eq!(
                get_remaining_points(&mut store, &instance),
                MeteringPoints::Remaining(SHA256_BASE_FUEL)
            );
            let sha = instance
                .exports
                .get_typed_function::<(u32, u32, u32), i32>(&store, "sha")
                .unwrap();
            assert_eq!(sha.call(&mut store, 0, 0, 64).unwrap(), 0);
            assert_eq!(
                get_remaining_points(&mut store, &instance),
                MeteringPoints::Remaining(0)
            );
            assert!(read.call(&mut store, 0).is_err());
            assert_eq!(
                get_remaining_points(&mut store, &instance),
                MeteringPoints::Exhausted
            );
            assert!(sha.call(&mut store, 0, 0, 64).is_err());
        }
    }
}
