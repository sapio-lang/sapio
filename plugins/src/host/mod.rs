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
use std::cell::Cell;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use wasmer::*;

pub mod plugin_handle;
pub mod wasm_cache;

/// The state that host-side functions need to be able to use
/// Also handles the imports of plugin-side functions
#[derive(WasmerEnv, Clone)]
pub struct HostEnvironmentInner {
    /// the module file path
    pub path: PathBuf,
    /// the currently running module's hash
    pub this: [u8; 32],
    /// a mapping of identifiers to module hashes
    pub module_map: BTreeMap<Vec<u8>, [u8; 32]>,
    /// the  global store of this runtime
    pub store: Arc<Mutex<Store>>,
    /// which network the contract is being built for
    pub net: bitcoin::Network,
    /// an emulator plugin for CTV functionality
    pub emulator: Arc<dyn CTVEmulator>,
    /// reference to the environment's memory space
    #[wasmer(export)]
    pub memory: LazyInit<Memory>,
    /// reference to allocation creation function
    #[wasmer(export(name = "sapio_v1_wasm_plugin_client_allocate_bytes"))]
    pub allocate_wasm_bytes: LazyInit<NativeFunc<i32, i32>>,
    /// reference to get_api function
    #[wasmer(export(name = "sapio_v1_wasm_plugin_client_get_create_arguments"))]
    pub get_api: LazyInit<NativeFunc<(), i32>>,
    /// reference to get_name function
    #[wasmer(export(name = "sapio_v1_wasm_plugin_client_get_name"))]
    pub get_name: LazyInit<NativeFunc<(), i32>>,
    /// reference to get_logo function
    #[wasmer(export(name = "sapio_v1_wasm_plugin_client_get_logo"))]
    pub get_logo: LazyInit<NativeFunc<(), i32>>,
    /// reference to allocation drop function
    #[wasmer(export(name = "sapio_v1_wasm_plugin_client_drop_allocation"))]
    pub forget: LazyInit<NativeFunc<i32, ()>>,
    /// reference to create function
    #[wasmer(export(name = "sapio_v1_wasm_plugin_client_create"))]
    pub create: LazyInit<NativeFunc<(i32, i32), i32>>,
    /// reference to entry point function
    #[wasmer(export(name = "sapio_v1_wasm_plugin_entry_point"))]
    pub init: LazyInit<NativeFunc<(), ()>>,
}

/// Wrapped Plugin Env so that we don't duplicate state for each function.
/// We must be careful to ensure we don't see deadlocks.
///
/// TODO: Figure out how to *just* make this Arc and not Mutex.
pub type HostEnvironment = Arc<Mutex<HostEnvironmentInner>>;

mod exports {
    //! the exports that the client will be able to use.
    //! They must be manually bound when instantiating the client.
    use std::str::FromStr;

    use crate::host::plugin_handle::SyncModuleLocator;

    use super::*;
    use sapio_base::effects::EffectPath;
    use sapio_data_repr::SapioModuleBoundaryRepr;
    /// lookup a plugin key from a human reable name.
    /// if ok == 1, result is valid.
    /// out is written and must be 32 bytes of writable memory.
    /// if name == 0 and name_len == 0, then return the current module
    pub fn sapio_v1_wasm_plugin_lookup_module_name(
        env: &HostEnvironment,
        key: i32,
        len: i32,
        out: i32,
        ok: i32,
    ) {
        let env = env.lock().unwrap();
        let m_hash = {
            if key == 0 && len == 0 {
                Some(&env.this)
            } else {
                let mut buf = vec![0u8; len as usize];
                for (src, dst) in env.memory_ref().unwrap().view()
                    [key as usize..(key + len) as usize]
                    .iter()
                    .map(Cell::get)
                    .zip(buf.iter_mut())
                {
                    *dst = src;
                }
                env.module_map.get(&buf)
            }
        };
        let is_ok = if let Some(b) = m_hash {
            let out = out as usize;
            for (src, dst) in b
                .iter()
                .zip(env.memory_ref().unwrap().view::<u8>()[out..out + 32].iter())
            {
                dst.set(*src);
            }
            1
        } else {
            0
        };
        env.memory_ref().unwrap().view::<u8>()[ok as usize].set(is_ok);
    }

    /// Create an instance of a contract by "trampolining" through the host to use another
    /// plugin identified by key.
    pub fn sapio_v1_wasm_plugin_get_api(env: &HostEnvironment, key: i32) -> i32 {
        wasm_plugin_action(env, key, Action::GetAPI)
    }
    /// Create an instance of a contract by "trampolining" through the host to use another
    /// plugin identified by key.
    pub fn sapio_v1_wasm_plugin_get_name(env: &HostEnvironment, key: i32) -> i32 {
        wasm_plugin_action(env, key, Action::GetName)
    }
    /// Create an instance of a contract by "trampolining" through the host to use another
    /// plugin identified by key.
    pub fn sapio_v1_wasm_plugin_get_logo(env: &HostEnvironment, key: i32) -> i32 {
        wasm_plugin_action(env, key, Action::GetLogo)
    }
    /// Create an instance of a contract by "trampolining" through the host to use another
    /// plugin identified by key.
    pub fn sapio_v1_wasm_plugin_create_contract(
        env: &HostEnvironment,
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

    fn wasm_plugin_action(env: &HostEnvironment, key: i32, action: Action) -> i32 {
        let env = env.lock().unwrap();
        const KEY_LEN: usize = 32;
        let key = key as usize;
        let h = wasmer_cache::Hash::new({
            let mut buf = [0u8; KEY_LEN];
            for (src, dst) in env.memory_ref().unwrap().view()[key..key + KEY_LEN]
                .iter()
                .map(Cell::get)
                .zip(buf.iter_mut())
            {
                *dst = src;
            }
            buf
        })
        .to_string();
        enum InternalAction {
            GetAPI,
            GetName,
            GetLogo,
            Create(CreateArgs<SapioModuleBoundaryRepr>, EffectPath),
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
                for (src, dst) in env.memory_ref().unwrap().view()
                    [json as usize..(json + json_len) as usize]
                    .iter()
                    .map(Cell::get)
                    .zip(v.iter_mut())
                {
                    *dst = src;
                }
                let create_args = sapio_data_repr::from_slice(&v[..json_len as usize])
                    .map_err(CompilationError::DeserializationError);

                for (src, dst) in env.memory_ref().unwrap().view()
                    [path as usize..(path + path_len) as usize]
                    .iter()
                    .map(Cell::get)
                    .zip(v.iter_mut())
                {
                    *dst = src;
                }
                let effectpath: Result<EffectPath, _> =
                    sapio_data_repr::from_slice(&v[..path_len as usize])
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
            WasmPluginHandle::<sapio_data_repr::SapioModuleBoundaryRepr>::new(
                path,
                &emulator,
                module_locator,
                net,
                Some(mmap),
            )
        }) {
            Ok(Ok(sph)) => {
                let comp_s = action_to_take.and_then(|action| match action {
                    InternalAction::GetName => sph.get_name().and_then(|m| {
                        sapio_data_repr::to_boundary_repr(&m)
                            .map_err(CompilationError::DeserializationError)
                    }),
                    InternalAction::GetLogo => sph.get_logo().and_then(|m| {
                        sapio_data_repr::to_boundary_repr(&m)
                            .map_err(CompilationError::DeserializationError)
                    }),
                    InternalAction::GetAPI => sph.get_api().and_then(|m| {
                        sapio_data_repr::to_boundary_repr(&m)
                            .map_err(CompilationError::DeserializationError)
                    }),
                    InternalAction::Create(create_args, path) => sph.call(&path, &create_args),
                });
                (move || -> Result<i32, CompilationError> {
                    // serialize the reuslt, not just the output.
                    let comp_s = sapio_data_repr::to_string(&comp_s.map_err(|s| s.to_string()))
                        .map_err(CompilationError::SerializationError)?;
                    let bytes: i32 = env
                        .allocate_wasm_bytes_ref()
                        .ok_or_else(|| {
                            CompilationError::ModuleCouldNotFindFunction(
                                "allocate_wasm_bytes".into(),
                            )
                        })?
                        .call(comp_s.len() as i32)
                        .map_err(|e| {
                            CompilationError::ModuleCouldNotAllocateError(
                                comp_s.len() as i32,
                                e.into(),
                            )
                        })?;
                    for (byte, c) in env.memory_ref().unwrap().view::<u8>()[bytes as usize..]
                        .iter()
                        .zip(comp_s.as_bytes())
                    {
                        byte.set(*c);
                    }
                    Ok(bytes)
                })()
                .unwrap_or(0)
            }
            _ => 0,
        }
    }

    /// use the hosts stdout to log a string. The host may make this a no-op.
    pub fn sapio_v1_wasm_plugin_debug_log_string(env: &HostEnvironment, a: i32, len: i32) {
        let env = env.lock().unwrap();
        let stdout = std::io::stdout();
        let lock = stdout.lock();
        let mut w = std::io::BufWriter::new(lock);
        let mem = env.memory_ref().unwrap().view::<u8>();
        for byte in mem[a as usize..(a + len) as usize].iter().map(Cell::get) {
            w.write_all(&[byte]).unwrap();
        }
        w.write_all("\n".as_bytes()).unwrap();
    }

    /// for the provided hash value, get the clause the oracle will satisfy
    pub fn sapio_v1_wasm_plugin_ctv_emulator_signer_for(env: &HostEnvironment, hash: i32) -> i32 {
        let env = env.lock().unwrap();
        let hash = hash as usize;
        let h = sha256::Hash::from_inner({
            let mut buf = [0u8; 32];
            for (src, dst) in env.memory_ref().unwrap().view()[hash..hash + 32]
                .iter()
                .map(Cell::get)
                .zip(buf.iter_mut())
            {
                *dst = src;
            }
            buf
        });
        let clause = env.emulator.get_signer_for(h).unwrap();
        let repr = sapio_data_repr::to_string(&clause).unwrap();
        let outbuf = env
            .allocate_wasm_bytes_ref()
            .unwrap()
            .call(repr.len() as i32)
            .unwrap();
        for (byte, c) in env.memory_ref().unwrap().view::<u8>()[outbuf as usize..]
            .iter()
            .zip(repr.as_bytes())
        {
            byte.set(*c);
        }
        outbuf
    }

    /// get the oracle to sign the psbt passed in
    pub fn sapio_v1_wasm_plugin_ctv_emulator_sign(
        env: &HostEnvironment,
        psbt: i32,
        len: u32,
    ) -> i32 {
        let env = env.lock().unwrap();
        let mut buf = vec![0u8; len as usize];
        let psbt = psbt as usize;
        for (src, dst) in env.memory_ref().unwrap().view()[psbt..]
            .iter()
            .map(Cell::get)
            .zip(buf.iter_mut())
        {
            *dst = src;
        }
        let psbt: PartiallySignedTransaction = sapio_data_repr::from_slice(&buf[..]).unwrap();
        let psbt = env.emulator.sign(psbt).unwrap();
        buf.clear();
        let repr = sapio_data_repr::to_string(&psbt).unwrap();
        let outbuf = env
            .allocate_wasm_bytes_ref()
            .unwrap()
            .call(repr.len() as i32)
            .unwrap();
        for (byte, c) in env.memory_ref().unwrap().view::<u8>()[outbuf as usize..]
            .iter()
            .zip(repr.as_bytes())
        {
            byte.set(*c);
        }
        outbuf
    }
}
