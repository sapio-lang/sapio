// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! host interface for modules

pub use crate::plugin_handle::PluginHandle;
use bitcoin::hashes::sha256;
use bitcoin::hashes::Hash;
use bitcoin::util::psbt::PartiallySignedTransaction;
pub use plugin_handle::WasmPluginHandle;
use sapio::contract::CompilationError;
use sapio_base::plugin_args::CreateArgs;
use sapio_ctv_emulator_trait::CTVEmulator;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use wasmer::*;

pub mod plugin_handle;
pub mod wasm_cache;

/// The state that host-side functions need to be able to use
/// Also handles the imports of plugin-side functions
#[derive(Clone)]
pub struct HostEnvironmentInner {
    /// the module file path
    pub path: PathBuf,
    /// the currently running module's hash
    pub this: [u8; 32],
    /// a mapping of identifiers to module hashes
    pub module_map: BTreeMap<Vec<u8>, [u8; 32]>,
    /// which network the contract is being built for
    pub net: bitcoin::Network,
    /// an emulator plugin for CTV functionality
    pub emulator: Arc<dyn CTVEmulator>,
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
    //! the exports that the client will be able to use.
    //! They must be manually bound when instantiating the client.
    use std::str::FromStr;

    use crate::host::plugin_handle::SyncModuleLocator;

    use super::*;
    use sapio_base::effects::EffectPath;
    /// lookup a plugin key from a human reable name.
    /// if ok == 1, result is valid.
    /// out is written and must be 32 bytes of writable memory.
    /// if name == 0 and name_len == 0, then return the current module
    pub fn sapio_v1_wasm_plugin_lookup_module_name(
        mut env: HostEnvironment,
        key: i32,
        len: i32,
        out: i32,
        ok: i32,
    ) {
        let (env, store) = env.data_and_store_mut();
        let m_hash = {
            if key == 0 && len == 0 {
                Some(&env.this)
            } else {
                let mut buf = vec![0u8; len as usize];
                env.memory
                    .as_ref()
                    .unwrap()
                    .view(&store)
                    .read(key as u64, &mut buf[..]);
                env.module_map.get(&buf)
            }
        };
        let is_ok = if let Some(b) = m_hash {
            env.memory
                .as_ref()
                .unwrap()
                .view(&store)
                .write(out as u64, b);
            1
        } else {
            0
        };
        env.memory
            .as_ref()
            .unwrap()
            .view(&store)
            .write_u8(ok as u64, is_ok);
    }

    /// Create an instance of a contract by "trampolining" through the host to use another
    /// plugin identified by key.
    pub fn sapio_v1_wasm_plugin_get_api(env: HostEnvironment, key: i32) -> i32 {
        wasm_plugin_action(env, key, Action::GetAPI)
    }
    /// Create an instance of a contract by "trampolining" through the host to use another
    /// plugin identified by key.
    pub fn sapio_v1_wasm_plugin_get_name(env: HostEnvironment, key: i32) -> i32 {
        wasm_plugin_action(env, key, Action::GetName)
    }
    /// Create an instance of a contract by "trampolining" through the host to use another
    /// plugin identified by key.
    pub fn sapio_v1_wasm_plugin_get_logo(env: HostEnvironment, key: i32) -> i32 {
        wasm_plugin_action(env, key, Action::GetLogo)
    }
    /// Create an instance of a contract by "trampolining" through the host to use another
    /// plugin identified by key.
    pub fn sapio_v1_wasm_plugin_create_contract(
        env: HostEnvironment,
        path: i32,
        path_len: i32,
        key: i32,
        json: i32,
        json_len: i32,
    ) -> i32 {
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

    fn wasm_plugin_action(mut env: HostEnvironment, key: i32, action: Action) -> i32 {
        let (env, mut store) = env.data_and_store_mut();
        const KEY_LEN: u64 = 32;
        let key = key as u64;
        let h = wasmer_cache::Hash::new({
            let mut buf = [0u8; KEY_LEN as usize];
            env.memory
                .as_ref()
                .unwrap()
                .view(&store)
                .read(key, &mut buf[..]);
            buf
        })
        .to_string();
        enum InternalAction {
            GetAPI,
            GetName,
            GetLogo,
            Create(CreateArgs<serde_json::Value>, EffectPath),
        }
        let action_to_take = match action {
            Action::GetAPI => Ok(InternalAction::GetAPI),
            Action::GetName => Ok(InternalAction::GetName),
            Action::GetLogo => Ok(InternalAction::GetLogo),
            Action::Create {
                path,
                path_len,
                json,
                json_len,
                ..
            } => {
                // use this buffer twice, so make it the max size
                let mut v = vec![0u8; std::cmp::max(json_len, path_len) as usize];
                env.memory
                    .as_ref()
                    .unwrap()
                    .view(&store)
                    .read(json as u64, &mut v[..json_len as usize]);
                let create_args = serde_json::from_str(
                    &String::from_utf8_lossy(&v[..json_len as usize]).to_owned(),
                )
                .map_err(CompilationError::DeserializationError);

                env.memory
                    .as_ref()
                    .unwrap()
                    .view(&store)
                    .read(path as u64, &mut v[..path_len as usize]);
                let effectpath: Result<EffectPath, _> = serde_json::from_str(
                    &String::from_utf8_lossy(&v[..path_len as usize]).to_owned(),
                )
                .map_err(CompilationError::DeserializationError);

                create_args.and_then(|c| effectpath.map(|e| InternalAction::Create(c, e)))
            }
        };
        let emulator = env.emulator.clone();
        let mmap = env.module_map.clone();
        let path = env.path.clone();
        let net = env.net;
        let key = wasmer_cache::Hash::from_str(&h).map(SyncModuleLocator::Key);
        // Use serde_json::Value for the WasmPluginHandle Output type
        match key.map(|module_locator| {
            WasmPluginHandle::<serde_json::Value>::new(
                path,
                &emulator,
                module_locator,
                net,
                Some(mmap),
            )
        }) {
            Ok(Ok(mut sph)) => {
                let comp_s = (move || -> Result<serde_json::Value, CompilationError> {
                    let value = match action_to_take? {
                        InternalAction::GetName => Ok(sph.get_name().and_then(|m| {
                            serde_json::to_value(m).map_err(CompilationError::DeserializationError)
                        })),
                        InternalAction::GetLogo => Ok(sph.get_logo().and_then(|m| {
                            serde_json::to_value(m).map_err(CompilationError::DeserializationError)
                        })),
                        InternalAction::GetAPI => Ok(sph.get_api().and_then(|m| {
                            serde_json::to_value(m).map_err(CompilationError::DeserializationError)
                        })),
                        InternalAction::Create(create_args, path) => {
                            sph.call(&path, &create_args).map(|comp| {
                                serde_json::to_value(comp)
                                    .map_err(CompilationError::DeserializationError)
                            })
                        }
                    };
                    value?
                })();
                (move || -> Result<i32, CompilationError> {
                    // serialize the reuslt, not just the output.
                    let comp_s = serde_json::to_string(&comp_s.map_err(|s| s.to_string()))
                        .map_err(CompilationError::SerializationError)?;
                    let bytes: i32 = env
                        .sapio_v1_wasm_plugin_client_allocate_bytes
                        .as_ref()
                        .ok_or_else(|| {
                            CompilationError::ModuleCouldNotFindFunction(
                                "allocate_wasm_bytes".into(),
                            )
                        })?
                        .call(&mut store, comp_s.len() as i32)
                        .map_err(|e| {
                            CompilationError::ModuleCouldNotAllocateError(
                                comp_s.len() as i32,
                                e.into(),
                            )
                        })?;
                    env.memory
                        .as_ref()
                        .unwrap()
                        .view(&store)
                        .write(bytes as u64, comp_s.as_bytes());
                    Ok(bytes)
                })()
                .unwrap_or(0)
            }
            _ => 0,
        }
    }

    /// use the hosts stdout to log a string. The host may make this a no-op.
    pub fn sapio_v1_wasm_plugin_debug_log_string(mut env: HostEnvironment, a: i32, len: i32) {
        let (env, store) = env.data_and_store_mut();
        let stdout = std::io::stdout();
        let lock = stdout.lock();
        let mut w = std::io::BufWriter::new(lock);
        let mem = env.memory.as_ref().unwrap().view(&store);
        let mut v = vec![0; len as usize];
        mem.read(a as u64, &mut v[..]);
        w.write_all(&v[..]);
        w.write_all("\n".as_bytes()).unwrap();
    }

    /// for the provided hash value, get the clause the oracle will satisfy
    pub fn sapio_v1_wasm_plugin_ctv_emulator_signer_for(
        mut env: HostEnvironment,
        hash: i32,
    ) -> i32 {
        let (env, mut store) = env.data_and_store_mut();
        let hash = hash as u64;
        let h = sha256::Hash::from_inner({
            let mut buf = [0u8; 32];
            env.memory
                .as_ref()
                .unwrap()
                .view(&store)
                .read(hash, &mut buf[..]);
            buf
        });
        let clause = env.emulator.get_signer_for(h).unwrap();
        let s = serde_json::to_string_pretty(&clause).unwrap();
        let bytes = env
            .sapio_v1_wasm_plugin_client_allocate_bytes
            .as_ref()
            .unwrap()
            .call(&mut store, s.len() as i32)
            .unwrap();
        env.memory
            .as_ref()
            .unwrap()
            .view(&store)
            .write(bytes as u64, s.as_bytes());
        bytes
    }

    /// get the oracle to sign the psbt passed in
    pub fn sapio_v1_wasm_plugin_ctv_emulator_sign(
        mut env: HostEnvironment,
        psbt: i32,
        len: u32,
    ) -> i32 {
        let (env, mut store) = env.data_and_store_mut();
        let mut buf = vec![0u8; len as usize];
        let psbt = psbt as u64;
        env.memory
            .as_ref()
            .unwrap()
            .view(&store)
            .read(psbt, &mut buf[..]);
        let psbt: PartiallySignedTransaction = serde_json::from_slice(&buf[..]).unwrap();
        let psbt = env.emulator.sign(psbt).unwrap();
        buf.clear();
        let s = serde_json::to_string_pretty(&psbt).unwrap();
        let bytes = env
            .sapio_v1_wasm_plugin_client_allocate_bytes
            .as_ref()
            .unwrap()
            .call(&mut store, s.len() as i32)
            .unwrap();
        env.memory
            .as_ref()
            .unwrap()
            .view(&store)
            .write(bytes as u64, s.as_bytes());
        bytes
    }
}
