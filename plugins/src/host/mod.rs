// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! host interface for modules

pub use crate::plugin_handle::PluginHandle;
pub use plugin_handle::WasmPluginHandle;
use sapio_base::plugin_args::CreateArgs;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use wasmer::*;

mod invocation;
mod memory;
pub mod plugin_handle;
mod runtime;
mod validation;
pub mod wasm_cache;

#[cfg(test)]
mod tests;

/// The state that host-side functions need to be able to use
/// Also handles the imports of plugin-side functions
#[derive(Clone)]
pub struct HostEnvironmentInner {
    /// Shared allowance for this instance's nested module calls.
    pub(crate) invocation_budget: invocation::InvocationBudget,
    /// Prevent recursive host callbacks from reentering the guest allocator.
    pub(crate) allocator_active: Arc<AtomicBool>,
    /// the module file path
    pub path: PathBuf,
    /// the currently running module's hash
    pub this: [u8; 32],
    /// a mapping of identifiers to module hashes
    pub module_map: BTreeMap<Vec<u8>, [u8; 32]>,
    /// which network the contract is being built for
    pub net: bitcoin::Network,
    /// reference to the environment's memory space
    pub memory: Option<Memory>,
    /// reference to allocation creation function
    pub sapio_v1_wasm_plugin_client_allocate_bytes: Option<TypedFunction<i32, i32>>,
    /// reference to get_api function
    pub sapio_v1_wasm_plugin_client_get_create_arguments: Option<TypedFunction<(), i32>>,
    /// reference to get_name function
    pub sapio_v1_wasm_plugin_client_get_name: Option<TypedFunction<(), i32>>,
    /// reference to get_logo function
    pub sapio_v1_wasm_plugin_client_get_logo: Option<TypedFunction<(), i32>>,
    /// reference to allocation drop function
    pub sapio_v1_wasm_plugin_client_drop_allocation: Option<TypedFunction<i32, ()>>,
    /// reference to create function
    pub sapio_v1_wasm_plugin_client_create: Option<TypedFunction<(i32, i32), i32>>,
    /// reference to entry point function
    pub sapio_v1_wasm_plugin_entry_point: Option<TypedFunction<(), ()>>,
}

/// Wrapped Plugin Env so that we don't duplicate state for each function.
/// We must be careful to ensure we don't see deadlocks.
///
/// TODO: Figure out how to *just* make this Arc and not Mutex.
pub type HostEnvironment<'a> = FunctionEnvMut<'a, HostEnvironmentInner>;
/// Bare HostEnvironment Type
pub type HostEnvironmentT = FunctionEnv<HostEnvironmentInner>;

mod exports {
    //! Imports exposed to a guest. Invalid ABI buffers trap the guest call.
    use super::memory::{read_buffer, runtime_error, write_buffer};
    use super::*;
    use crate::host::plugin_handle::SyncModuleLocator;
    use sapio_base::effects::EffectPath;

    fn guest_memory<'a>(
        env: &'a HostEnvironmentInner,
        store: &'a impl AsStoreRef,
    ) -> Result<MemoryView<'a>, RuntimeError> {
        Ok(env
            .memory
            .as_ref()
            .ok_or_else(|| runtime_error("WASM guest memory is not initialized"))?
            .view(store))
    }

    fn return_string(
        env: &HostEnvironmentInner,
        store: &mut impl AsStoreMut,
        value: &str,
    ) -> Result<i32, RuntimeError> {
        memory::check_length(value.len().saturating_add(1))?;
        let allocate = env
            .sapio_v1_wasm_plugin_client_allocate_bytes
            .as_ref()
            .ok_or_else(|| runtime_error("WASM guest allocator is not initialized"))?;
        if env.allocator_active.swap(true, Ordering::Relaxed) {
            return Err(runtime_error("WASM guest allocator reentry is not allowed"));
        }
        let allocation = allocate.call(store, value.len() as i32);
        env.allocator_active.store(false, Ordering::Relaxed);
        let ptr = allocation?;
        memory::write_string(&guest_memory(env, store)?, ptr, value)?;
        Ok(ptr)
    }

    /// Look up a module key. A null, empty name selects the current module.
    /// Write the 32-byte key and set `ok` to one when the name is found.
    pub fn sapio_v1_wasm_plugin_lookup_module_name(
        mut env: HostEnvironment,
        key: i32,
        len: i32,
        out: i32,
        ok: i32,
    ) -> Result<(), RuntimeError> {
        let (env, store) = env.data_and_store_mut();
        let memory = guest_memory(env, &store)?;
        let hash = if key == 0 && len == 0 {
            Some(&env.this)
        } else {
            let name = read_buffer(&memory, key, len as u32 as usize)?;
            env.module_map.get(&name)
        };
        if let Some(hash) = hash {
            write_buffer(&memory, out, hash)?;
        }
        write_buffer(&memory, ok, &[u8::from(hash.is_some())])
    }

    /// Retrieve another module's API.
    pub fn sapio_v1_wasm_plugin_get_api(
        env: HostEnvironment,
        key: i32,
    ) -> Result<i32, RuntimeError> {
        wasm_plugin_action(env, key, Action::GetAPI)
    }
    /// Retrieve another module's name.
    pub fn sapio_v1_wasm_plugin_get_name(
        env: HostEnvironment,
        key: i32,
    ) -> Result<i32, RuntimeError> {
        wasm_plugin_action(env, key, Action::GetName)
    }
    /// Retrieve another module's logo.
    pub fn sapio_v1_wasm_plugin_get_logo(
        env: HostEnvironment,
        key: i32,
    ) -> Result<i32, RuntimeError> {
        wasm_plugin_action(env, key, Action::GetLogo)
    }
    /// Compile a contract using another module identified by its key.
    pub fn sapio_v1_wasm_plugin_create_contract(
        env: HostEnvironment,
        path: i32,
        path_len: i32,
        key: i32,
        json: i32,
        json_len: i32,
    ) -> Result<i32, RuntimeError> {
        wasm_plugin_action(
            env,
            key,
            Action::Create {
                path,
                path_len,
                json,
                json_len,
            },
        )
    }

    enum Action {
        Create {
            path: i32,
            path_len: i32,
            json: i32,
            json_len: i32,
        },
        GetAPI,
        GetName,
        GetLogo,
    }

    fn wasm_plugin_action(
        mut env: HostEnvironment,
        key: i32,
        action: Action,
    ) -> Result<i32, RuntimeError> {
        let (env, mut store) = env.data_and_store_mut();
        let mut key_bytes = [0; 32];
        key_bytes.copy_from_slice(&read_buffer(&guest_memory(env, &store)?, key, 32)?);
        let create = if let Action::Create {
            path,
            path_len,
            json,
            json_len,
        } = action
        {
            let memory = guest_memory(env, &store)?;
            let arguments: CreateArgs<serde_json::Value> =
                serde_json::from_slice(&read_buffer(&memory, json, json_len as u32 as usize)?)
                    .map_err(runtime_error)?;
            arguments
                .context
                .lowering
                .validate()
                .map_err(runtime_error)?;
            let path: EffectPath =
                serde_json::from_slice(&read_buffer(&memory, path, path_len as u32 as usize)?)
                    .map_err(runtime_error)?;
            Some((arguments, path))
        } else {
            None
        };
        let budget = env.invocation_budget.child()?;
        // Ordinary module errors retain the v1 Result JSON representation.
        // Invalid memory and malformed inputs trap before loading another module.
        let result = (|| -> Result<serde_json::Value, String> {
            let mut plugin = WasmPluginHandle::<serde_json::Value>::new_with_budget(
                env.path.clone(),
                SyncModuleLocator::Key(wasmer_cache::Hash::new(key_bytes)),
                env.net,
                Some(env.module_map.clone()),
                budget,
            )
            .map_err(|error| error.to_string())?;
            match action {
                Action::GetAPI => {
                    serde_json::to_value(plugin.get_api().map_err(|error| error.to_string())?)
                }
                Action::GetName => {
                    serde_json::to_value(plugin.get_name().map_err(|error| error.to_string())?)
                }
                Action::GetLogo => {
                    serde_json::to_value(plugin.get_logo().map_err(|error| error.to_string())?)
                }
                Action::Create { .. } => {
                    let (arguments, path) = create.ok_or("Missing contract arguments")?;
                    return plugin
                        .call(&path, &arguments)
                        .map_err(|error| error.to_string());
                }
            }
            .map_err(|error| error.to_string())
        })();
        return_string(
            env,
            &mut store,
            &serde_json::to_string(&result).map_err(runtime_error)?,
        )
    }

    /// Write a guest diagnostic to stderr, preserving stdout for machine output.
    pub fn sapio_v1_wasm_plugin_debug_log_string(
        mut env: HostEnvironment,
        ptr: i32,
        len: i32,
    ) -> Result<(), RuntimeError> {
        let (env, store) = env.data_and_store_mut();
        let bytes = read_buffer(&guest_memory(env, &store)?, ptr, len as u32 as usize)?;
        let mut stderr = std::io::stderr().lock();
        stderr.write_all(&bytes).map_err(runtime_error)?;
        stderr.write_all(b"\n").map_err(runtime_error)
    }
}
