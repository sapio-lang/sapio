// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//!  a plugin handle for a wasm plugin.
use super::*;
use crate::host::wasm_cache::get_all_keys_from_fs;
use crate::host::HostEnvironmentInner;
use crate::host::{exports::*, HostEnvironmentT};
use crate::plugin_handle::PluginHandle;
use crate::API;
use sapio::contract::CompilationError;
use sapio_base::effects::EffectPath;
use sapio_ctv_emulator_trait::CTVEmulator;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::marker::PhantomData;
use std::path::PathBuf;
use wasmer::{FunctionEnv, Memory, TypedFunction};

/// Helper to resolve modules
#[derive(Serialize, Deserialize, JsonSchema)]
pub enum ModuleLocator {
    /// A Hex Encoded Hash
    Key(String),
    /// A File Name of an uncompiled WASM Module
    FileName(String),
    /// The Raw Uncompiled Bytes of a Module
    Bytes(Vec<u8>),
    /// Not Known
    Unknown,
}

impl ModuleLocator {
    async fn locate(self) -> Result<SyncModuleLocator, Box<dyn std::error::Error>> {
        match self {
            ModuleLocator::Key(k) => {
                let key = WASMCacheID::from_str(&k)?;
                Ok(SyncModuleLocator::Key(key))
            }
            ModuleLocator::FileName(f) => Ok(SyncModuleLocator::Bytes(tokio::fs::read(f).await?)),
            ModuleLocator::Bytes(b) => Ok(SyncModuleLocator::Bytes(b)),
            ModuleLocator::Unknown => Err(Err(CompilationError::UnknownModule)?),
        }
    }
}
/// After a module has been located, we can only be either new bytes read from
/// somewhere or a hash already in our cache.
pub enum SyncModuleLocator {
    /// Module is in Cache
    Key(wasmer_cache::Hash),
    /// Module is here
    Bytes(Vec<u8>),
}

/// A handle that holds a WASM Module instance
pub struct WasmPluginHandle<Output> {
    store: Store,
    env: HostEnvironmentT,
    module: Module,
    instance: Instance,
    key: wasmer_cache::Hash,
    net: bitcoin::Network,
    _pd: PhantomData<Output>,
    /// reference to allocation creation function
    pub sapio_v1_wasm_plugin_client_allocate_bytes: TypedFunction<i32, i32>,
    /// reference to get_api function
    pub sapio_v1_wasm_plugin_client_get_create_arguments: TypedFunction<(), i32>,
    /// reference to get_name function
    pub sapio_v1_wasm_plugin_client_get_name: TypedFunction<(), i32>,
    /// reference to get_logo function
    pub sapio_v1_wasm_plugin_client_get_logo: TypedFunction<(), i32>,
    /// reference to allocation drop function
    pub sapio_v1_wasm_plugin_client_drop_allocation: TypedFunction<i32, ()>,
    /// reference to create function
    pub sapio_v1_wasm_plugin_client_create: TypedFunction<(i32, i32), i32>,
    /// reference to entry point function
    pub sapio_v1_wasm_plugin_entry_point: TypedFunction<(), ()>,
}

impl<T> WasmPluginHandle<T> {
    /// Clone with a new memory space/instance
    pub fn fresh_clone(&self) -> Result<Self, Box<dyn Error>> {
        let env = self.env.as_ref(&self.store);
        Ok(Self::setup_plugin_inner(
            Store::default(),
            env.path.clone(),
            env.this,
            Some(env.module_map.clone()),
            self.net,
            &env.emulator,
            self.module.clone(),
            self.key,
        )?)
    }
}
impl<Output> WasmPluginHandle<Output> {
    /// the cache ID for this plugin
    pub fn id(&self) -> WASMCacheID {
        self.key
    }

    /// load all the cached keys as plugins upfront.
    pub fn load_all_keys<I: Into<PathBuf> + Clone>(
        path: I,
        emulator: NullEmulator,
        net: bitcoin::Network,
        plugin_map: Option<BTreeMap<Vec<u8>, [u8; 32]>>,
    ) -> Result<Vec<Self>, Box<dyn Error>> {
        let mut r = vec![];
        for key in get_all_keys_from_fs(path.clone())? {
            let wph = Self::new(
                path.clone(),
                &emulator,
                SyncModuleLocator::Key(WASMCacheID::from_str(&key)?),
                net,
                plugin_map.clone(),
            )?;
            r.push(wph)
        }
        Ok(r)
    }

    /// Create a new module using async module resolution
    pub async fn new_async<I: Into<PathBuf> + Clone>(
        path: I,
        emulator: &Arc<dyn CTVEmulator>,
        module_locator: ModuleLocator,
        net: bitcoin::Network,
        plugin_map: Option<BTreeMap<Vec<u8>, [u8; 32]>>,
    ) -> Result<Self, Box<dyn Error>> {
        Self::new(
            path,
            emulator,
            module_locator.locate().await?,
            net,
            plugin_map,
        )
    }
    /// Create an plugin handle. Only one of key or file should be set, and one
    /// should be set.
    /// TODO: Revert to async?
    pub fn new<I: Into<PathBuf> + Clone>(
        path: I,
        emulator: &Arc<dyn CTVEmulator>,
        module_locator: SyncModuleLocator,
        net: bitcoin::Network,
        plugin_map: Option<BTreeMap<Vec<u8>, [u8; 32]>>,
    ) -> Result<Self, Box<dyn Error>> {
        let store = Store::default();

        let (module, key) = load_module_from_cache(module_locator, &path, &store)?;

        let mut this = [0; 32];
        this.clone_from_slice(&hex::decode(key.to_string())?);
        Self::setup_plugin_inner(store, path, this, plugin_map, net, emulator, module, key)
    }

    /// forget an allocated pointer
    pub fn forget(&mut self, p: i32) -> Result<(), CompilationError> {
        self.sapio_v1_wasm_plugin_client_drop_allocation
            .call(&mut self.store, p)
            .map_err(|e| CompilationError::ModuleCouldNotDeallocate(p, e.into()))
    }

    /// create an allocation
    pub fn allocate(&mut self, len: i32) -> Result<i32, CompilationError> {
        self.sapio_v1_wasm_plugin_client_allocate_bytes
            .call(&mut self.store, len)
            .map_err(|e| CompilationError::ModuleCouldNotAllocateError(len, e.into()))
    }

    /// pass a string to the WASM plugin
    pub fn pass_string(&mut self, s: &str) -> Result<i32, CompilationError> {
        let offset = self.allocate(s.len() as i32)?;
        match self.pass_string_inner(s, offset) {
            Ok(_) => Ok(offset),
            Err(e) => {
                self.forget(offset)?;
                Err(e)
            }
        }
    }

    /// helper for string passing
    fn pass_string_inner(&self, s: &str, offset: i32) -> Result<(), CompilationError> {
        let env = self.env.as_ref(&self.store);
        env.memory
            .as_ref()
            .ok_or(CompilationError::ModuleFailedToGetMemory(
                "Memory Missing".into(),
            ))?
            .view(&self.store)
            .write(offset as u64, &s.as_bytes()[..]);
        Ok(())
    }

    fn get_memory(&self) -> Result<&Memory, CompilationError> {
        self.instance
            .exports
            .get_memory("memory")
            .map_err(|e| CompilationError::ModuleFailedToGetMemory(e.into()))
    }
    /// read something from wasm memory, null terminated
    fn read_to_vec(&self, p: i32) -> Result<Vec<u8>, CompilationError> {
        let env = self.env.as_ref(&self.store);
        let mem = env
            .memory
            .as_ref()
            .ok_or(CompilationError::ModuleFailedToGetMemory(
                "Memory Missing".into(),
            ))?
            .view(&self.store);
        let p = p as u64;
        let mut e = p;
        while mem.read_u8(e).ok() != Some(0) {
            e += 1;
        }
        let mut v = vec![0; (e - p) as usize];
        mem.read(p, &mut v[..]);
        Ok(v)
    }

    fn setup_plugin_inner<I: Into<PathBuf> + Clone>(
        mut store: Store,
        path: I,
        this: [u8; 32],
        plugin_map: Option<BTreeMap<Vec<u8>, [u8; 32]>>,
        net: bitcoin::Network,
        emulator: &Arc<dyn CTVEmulator>,
        module: Module,
        key: WASMCacheID,
    ) -> Result<Self, Box<dyn Error>> {
        let host_env = FunctionEnv::new(
            &mut store,
            HostEnvironmentInner {
                path: path.into(),
                this,
                module_map: plugin_map.unwrap_or_default(),
                net,
                emulator: emulator.clone(),
                memory: None,
                sapio_v1_wasm_plugin_client_get_create_arguments: None,
                sapio_v1_wasm_plugin_client_get_name: None,
                sapio_v1_wasm_plugin_client_get_logo: None,
                sapio_v1_wasm_plugin_client_drop_allocation: None,
                sapio_v1_wasm_plugin_client_create: None,
                sapio_v1_wasm_plugin_entry_point: None,
                sapio_v1_wasm_plugin_client_allocate_bytes: None,
            },
        );
        macro_rules! create_imports {
        ($store:ident, $env:ident $(,$names:ident)*) =>
        {
            imports! {
                "env" =>  {
                    $( std::stringify!($names) => Function::new_typed_with_env( &mut $store, &$env, $names) ,)*
                }
            }
        };
    }
        // grab data and a new store_mut
        let import_object = create_imports!(
            store,
            host_env,
            sapio_v1_wasm_plugin_ctv_emulator_signer_for,
            sapio_v1_wasm_plugin_ctv_emulator_sign,
            sapio_v1_wasm_plugin_debug_log_string,
            sapio_v1_wasm_plugin_create_contract,
            sapio_v1_wasm_plugin_get_api,
            sapio_v1_wasm_plugin_get_name,
            sapio_v1_wasm_plugin_get_logo,
            sapio_v1_wasm_plugin_lookup_module_name
        );
        let instance = Instance::new(&mut store, &module, &import_object)?;
        let mut env_mut = host_env.into_mut(&mut store);
        // change to a FunctionEnvMut
        let (data_mut, mut store_mut) = env_mut.data_and_store_mut();

        data_mut.memory = Some(instance.exports.get_memory("memory")?.clone());
        macro_rules! create_exports {
        ($store:ident, $env:ident, $instance:ident $(,$names:ident)*) =>
        {
            $($env.$names = Some($instance.exports.get_typed_function(&mut $store, std::stringify!($names))?);)*
        };
    }
        create_exports!(
            store_mut,
            data_mut,
            instance,
            sapio_v1_wasm_plugin_client_allocate_bytes,
            sapio_v1_wasm_plugin_client_get_create_arguments,
            sapio_v1_wasm_plugin_client_get_name,
            sapio_v1_wasm_plugin_client_get_logo,
            sapio_v1_wasm_plugin_client_drop_allocation,
            sapio_v1_wasm_plugin_client_create,
            sapio_v1_wasm_plugin_entry_point
        );

        data_mut
            .sapio_v1_wasm_plugin_entry_point
            .as_ref()
            .ok_or("No Init Function Specified")?
            .call(&mut store_mut)?;

        macro_rules! create_handle_exports {
        ($store:ident,  $instance:ident $(,$names:ident)*) =>
        {
            $(let $names = $instance.exports.get_typed_function(&mut $store, std::stringify!($names))?;)*
        }
    }
        create_handle_exports!(
            store_mut,
            instance,
            sapio_v1_wasm_plugin_client_allocate_bytes,
            sapio_v1_wasm_plugin_client_get_create_arguments,
            sapio_v1_wasm_plugin_client_get_name,
            sapio_v1_wasm_plugin_client_get_logo,
            sapio_v1_wasm_plugin_client_drop_allocation,
            sapio_v1_wasm_plugin_client_create,
            sapio_v1_wasm_plugin_entry_point
        );
        Ok(WasmPluginHandle {
            sapio_v1_wasm_plugin_client_allocate_bytes,
            sapio_v1_wasm_plugin_client_get_create_arguments,
            sapio_v1_wasm_plugin_client_get_name,
            sapio_v1_wasm_plugin_client_get_logo,
            sapio_v1_wasm_plugin_client_drop_allocation,
            sapio_v1_wasm_plugin_client_create,
            sapio_v1_wasm_plugin_entry_point,
            env: env_mut.as_ref(),
            store,
            net,
            module,
            instance,
            key,
            _pd: Default::default(),
        })
    }
}
fn load_module_from_cache<I: Into<PathBuf> + Clone>(
    module_locator: SyncModuleLocator,
    path: &I,
    store: &Store,
) -> Result<(Module, WASMCacheID), Box<dyn Error>> {
    let (module, key) = match module_locator {
        SyncModuleLocator::Bytes(wasm_bytes) => {
            match wasm_cache::load_module(path.clone(), store, &wasm_bytes[..]) {
                Ok(module) => module,
                Err(_) => {
                    let module = Module::new(&store, &wasm_bytes)?;
                    let key = wasm_cache::store_module(path.clone(), &module, &wasm_bytes)?;
                    (module, key)
                }
            }
        }
        SyncModuleLocator::Key(key) => wasm_cache::load_module_key(path.clone(), store, key)?,
    };
    Ok((module, key))
}

impl<GOutput> PluginHandle for WasmPluginHandle<GOutput>
where
    GOutput: for<'a> Deserialize<'a>,
{
    type Input = CreateArgs<serde_json::Value>;
    type Output = GOutput;
    fn call(
        &mut self,
        path: &EffectPath,
        c: &Self::Input,
    ) -> Result<Self::Output, CompilationError> {
        let arg_str = serde_json::to_string(c).map_err(CompilationError::SerializationError)?;
        let args_ptr = self.pass_string(&arg_str)?;
        let path_str = serde_json::to_string(path).map_err(CompilationError::SerializationError)?;
        let path_ptr = self.pass_string(&path_str)?;
        let _env = self.env.as_mut(&mut self.store);
        let create_func = { &self.sapio_v1_wasm_plugin_client_create };
        let result_ptr = create_func
            .call(&mut self.store, path_ptr, args_ptr)
            .map_err(|e| {
                CompilationError::ModuleCouldNotCreateContract(path.clone(), c.clone(), e.into())
            })?;
        let buf = self.read_to_vec(result_ptr)?;
        self.forget(result_ptr)?;
        let v: Result<Self::Output, String> =
            serde_json::from_slice(&buf).map_err(CompilationError::DeserializationError)?;
        v.map_err(CompilationError::ModuleCompilationErrorUnsendable)
    }
    fn get_api(&mut self) -> Result<API<Self::Input, Self::Output>, CompilationError> {
        let _env = self.env.as_mut(&mut self.store);
        let p = self
            .sapio_v1_wasm_plugin_client_get_create_arguments
            .call(&mut self.store)
            .map_err(|e| CompilationError::ModuleCouldNotGetAPI(e.into()))?;
        let v = self.read_to_vec(p)?;
        self.forget(p)?;
        serde_json::from_slice(&v).map_err(CompilationError::DeserializationError)
    }
    fn get_name(&mut self) -> Result<String, CompilationError> {
        let _env = self.env.as_mut(&mut self.store);
        let p = self
            .sapio_v1_wasm_plugin_client_get_name
            .call(&mut self.store)
            .map_err(|e| CompilationError::ModuleCouldNotGetName(e.into()))?;
        let v = self.read_to_vec(p)?;
        self.forget(p)?;
        Ok(String::from_utf8_lossy(&v).to_string())
    }

    fn get_logo(&mut self) -> Result<String, CompilationError> {
        let _env = self.env.as_mut(&mut self.store);
        let p = self
            .sapio_v1_wasm_plugin_client_get_logo
            .call(&mut self.store)
            .map_err(|e| CompilationError::ModuleCouldNotGetLogo(e.into()))?;
        let v = self.read_to_vec(p)?;
        self.forget(p)?;
        Ok(String::from_utf8_lossy(&v).to_string())
    }
}
