// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//!  a plugin handle for a wasm plugin.
use super::*;
use crate::host::invocation::InvocationBudget;
use crate::host::memory::{self, runtime_error};
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
use std::sync::atomic::AtomicBool;
use tokio::io::AsyncReadExt;
use wasmer::{FunctionEnv, TypedFunction};

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
            ModuleLocator::FileName(f) => {
                let limit = wasm_cache::MAX_MODULE_BYTES as u64;
                let check_file = |metadata: std::fs::Metadata| {
                    if !metadata.is_file() || metadata.len() > limit {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "WASM module must be a regular file no larger than 128 MiB",
                        ))
                    } else {
                        Ok(())
                    }
                };
                check_file(tokio::fs::metadata(&f).await?)?;
                let file = tokio::fs::File::open(f).await?;
                check_file(file.metadata().await?)?;
                let mut bytes = Vec::new();
                file.take(limit + 1).read_to_end(&mut bytes).await?;
                if bytes.len() > limit as usize {
                    return Err("WASM source exceeds the module size limit".into());
                }
                Ok(SyncModuleLocator::Bytes(bytes))
            }
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
    _instance: Instance,
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
        // The compiled code and signature IDs belong to the original engine.
        Ok(Self::setup_plugin_inner(
            Store::new(self.store.engine().clone()),
            env.path.clone(),
            env.this,
            Some(env.module_map.clone()),
            self.net,
            &env.emulator,
            self.module.clone(),
            self.key,
            InvocationBudget::new(),
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
        Self::new_with_budget(
            path,
            emulator,
            module_locator,
            net,
            plugin_map,
            InvocationBudget::new(),
        )
    }

    pub(in crate::host) fn new_with_budget<I: Into<PathBuf> + Clone>(
        path: I,
        emulator: &Arc<dyn CTVEmulator>,
        module_locator: SyncModuleLocator,
        net: bitcoin::Network,
        plugin_map: Option<BTreeMap<Vec<u8>, [u8; 32]>>,
        invocation_budget: InvocationBudget,
    ) -> Result<Self, Box<dyn Error>> {
        let store = crate::host::runtime::new_store();

        let (module, key) = load_module_from_cache(module_locator, &path, &store)?;

        let mut this = [0; 32];
        this.clone_from_slice(&hex::decode(key.to_string())?);
        Self::setup_plugin_inner(
            store,
            path,
            this,
            plugin_map,
            net,
            emulator,
            module,
            key,
            invocation_budget,
        )
    }

    /// forget an allocated pointer
    pub fn forget(&mut self, p: i32) -> Result<(), CompilationError> {
        self.sapio_v1_wasm_plugin_client_drop_allocation
            .call(&mut self.store, p)
            .map_err(|e| CompilationError::ModuleCouldNotDeallocate(p, e.into()))
    }

    /// create an allocation
    pub fn allocate(&mut self, len: i32) -> Result<i32, CompilationError> {
        memory::check_length((len as u32 as usize).saturating_add(1))
            .map_err(|error| CompilationError::ModuleRuntimeError(error.into()))?;
        self.sapio_v1_wasm_plugin_client_allocate_bytes
            .call(&mut self.store, len)
            .map_err(|e| CompilationError::ModuleCouldNotAllocateError(len, e.into()))
    }

    /// pass a string to the WASM plugin
    pub fn pass_string(&mut self, s: &str) -> Result<i32, CompilationError> {
        memory::check_length(s.len().saturating_add(1))
            .map_err(|error| CompilationError::ModuleRuntimeError(error.into()))?;
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
        let memory = env
            .memory
            .as_ref()
            .ok_or(CompilationError::ModuleFailedToGetMemory(
                "Memory Missing".into(),
            ))?
            .view(&self.store);
        memory::write_string(&memory, offset, s)
            .map_err(|error| CompilationError::ModuleRuntimeError(error.into()))
    }

    /// Read a bounded, null-terminated string from guest memory.
    fn read_to_vec(&self, ptr: i32) -> Result<Vec<u8>, CompilationError> {
        let env = self.env.as_ref(&self.store);
        let memory = env
            .memory
            .as_ref()
            .ok_or(CompilationError::ModuleFailedToGetMemory(
                "Memory Missing".into(),
            ))?
            .view(&self.store);
        memory::read_string(&memory, ptr)
            .map_err(|error| CompilationError::ModuleRuntimeError(error.into()))
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
        invocation_budget: InvocationBudget,
    ) -> Result<Self, Box<dyn Error>> {
        let host_env = FunctionEnv::new(
            &mut store,
            HostEnvironmentInner {
                invocation_budget,
                allocator_active: Arc::new(AtomicBool::new(false)),
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
            _instance: instance,
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
    match module_locator {
        SyncModuleLocator::Bytes(wasm_bytes) => {
            wasm_cache::load_module(path.clone(), store, &wasm_bytes)
        }
        SyncModuleLocator::Key(key) => wasm_cache::load_module_key(path.clone(), store, key),
    }
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
        String::from_utf8(v)
            .map_err(|error| CompilationError::ModuleRuntimeError(runtime_error(error).into()))
    }

    fn get_logo(&mut self) -> Result<String, CompilationError> {
        let _env = self.env.as_mut(&mut self.store);
        let p = self
            .sapio_v1_wasm_plugin_client_get_logo
            .call(&mut self.store)
            .map_err(|e| CompilationError::ModuleCouldNotGetLogo(e.into()))?;
        let v = self.read_to_vec(p)?;
        self.forget(p)?;
        String::from_utf8(v)
            .map_err(|error| CompilationError::ModuleRuntimeError(runtime_error(error).into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sapio_ctv_emulator_trait::CTVAvailable;
    use wasmer_middlewares::metering::{get_remaining_points, MeteringPoints};

    fn plugin(
        name_body: &str,
        extra: &str,
        allocation: i32,
    ) -> WasmPluginHandle<serde_json::Value> {
        load_plugin(&plugin_source(name_body, extra, allocation)).unwrap()
    }

    fn plugin_source(name_body: &str, extra: &str, allocation: i32) -> String {
        format!(
            r#"(module
                (import "env" "sapio_v1_wasm_plugin_debug_log_string"
                    (func $log (param i32 i32)))
                (import "env" "sapio_v1_wasm_plugin_ctv_emulator_sign"
                    (func $sign (param i32 i32) (result i32)))
                (import "env" "sapio_v1_wasm_plugin_lookup_module_name"
                    (func $lookup (param i32 i32 i32 i32)))
                (memory (export "memory") 1)
                (data (i32.const 8) "ok\00")
                (func (export "sapio_v1_wasm_plugin_client_allocate_bytes")
                    (param i32) (result i32) i32.const {allocation})
                (func (export "sapio_v1_wasm_plugin_client_get_create_arguments")
                    (result i32) i32.const 8)
                (func (export "sapio_v1_wasm_plugin_client_get_name")
                    (result i32) {name_body})
                (func (export "sapio_v1_wasm_plugin_client_get_logo")
                    (result i32) i32.const 8)
                (func (export "sapio_v1_wasm_plugin_client_drop_allocation") (param i32))
                (func (export "sapio_v1_wasm_plugin_client_create")
                    (param i32 i32) (result i32) i32.const 8)
                (func (export "sapio_v1_wasm_plugin_entry_point"))
                {extra}
            )"#
        )
    }

    fn load_plugin(source: &str) -> Result<WasmPluginHandle<serde_json::Value>, Box<dyn Error>> {
        let store = crate::host::runtime::new_store();
        let module = Module::new(&store, source.as_bytes())?;
        let emulator: Arc<dyn CTVEmulator> = Arc::new(CTVAvailable);
        WasmPluginHandle::setup_plugin_inner(
            store,
            PathBuf::from("."),
            [0; 32],
            None,
            bitcoin::Network::Regtest,
            &emulator,
            module,
            WASMCacheID::generate(source.as_bytes()),
            InvocationBudget::new(),
        )
    }

    fn spinning_export(name: &str, signature: &str) -> String {
        let export = format!("sapio_v1_wasm_plugin_{name}");
        let mut source =
            plugin_source("i32.const 8", "", 16).replacen(&format!("(export \"{export}\")"), "", 1);
        let end = source.rfind(')').unwrap();
        source.insert_str(
            end,
            &format!("(func (export \"{export}\") {signature} (loop br 0) unreachable)"),
        );
        source
    }

    #[test]
    fn infinite_wasm_start_and_entrypoint_exhaust_fuel_before_setup_completes() {
        let source = plugin_source(
            "i32.const 8",
            "(func $start (loop br 0)) (start $start)",
            16,
        );
        let error = load_plugin(&source)
            .err()
            .expect("infinite start must trap");
        assert!(
            matches!(
                error.downcast_ref::<wasmer::InstantiationError>(),
                Some(wasmer::InstantiationError::Start(_))
            ),
            "{error}"
        );

        let error = load_plugin(&spinning_export("entry_point", ""))
            .err()
            .expect("infinite entrypoint must trap");
        assert!(
            error.downcast_ref::<wasmer::RuntimeError>().is_some(),
            "{error}"
        );
    }

    #[test]
    fn infinite_guest_code_traps_on_every_public_call_path() {
        let arguments = CreateArgs {
            arguments: serde_json::Value::Null,
            context: crate::ContextualArguments {
                network: bitcoin::Network::Regtest,
                amount: bitcoin::Amount::from_sat(0),
                effects: Default::default(),
                ordinals_info: None,
            },
        };
        let path = EffectPath::try_from("metering").unwrap();
        for (export, signature) in [
            ("client_get_name", "(result i32)"),
            ("client_get_logo", "(result i32)"),
            ("client_get_create_arguments", "(result i32)"),
            ("client_create", "(param i32 i32) (result i32)"),
            ("client_allocate_bytes", "(param i32) (result i32)"),
            ("client_drop_allocation", "(param i32)"),
        ] {
            let mut plugin = load_plugin(&spinning_export(export, signature)).unwrap();
            let failed = match export {
                "client_get_name" => plugin.get_name().is_err(),
                "client_get_logo" => plugin.get_logo().is_err(),
                "client_get_create_arguments" => plugin.get_api().is_err(),
                "client_create" => plugin.call(&path, &arguments).is_err(),
                "client_allocate_bytes" => plugin.pass_string("guest input").is_err(),
                // Retrieval invokes cleanup after reading a valid result.
                "client_drop_allocation" => plugin.get_name().is_err(),
                _ => unreachable!(),
            };
            assert!(failed, "{export} must trap");
            assert_eq!(
                get_remaining_points(&mut plugin.store, &plugin._instance),
                MeteringPoints::Exhausted,
                "{export} must fail from fuel exhaustion"
            );
        }
    }

    struct FixtureDirectory(PathBuf);

    impl FixtureDirectory {
        fn new() -> Self {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "sapio-metered-plugin-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for FixtureDirectory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn remaining_fuel(plugin: &mut WasmPluginHandle<serde_json::Value>) -> u64 {
        match get_remaining_points(&mut plugin.store, &plugin._instance) {
            MeteringPoints::Remaining(fuel) => fuel,
            MeteringPoints::Exhausted => panic!("expected unspent fuel"),
        }
    }

    fn exhaust_finite_calls(plugin: &mut WasmPluginHandle<serde_json::Value>) {
        for _ in 0..10 {
            let before = remaining_fuel(plugin);
            match plugin.get_name() {
                Ok(name) => {
                    assert_eq!(name, "ok");
                    assert!(remaining_fuel(plugin) < before);
                }
                Err(_) => {
                    assert_eq!(
                        get_remaining_points(&mut plugin.store, &plugin._instance),
                        MeteringPoints::Exhausted
                    );
                    return;
                }
            }
        }
        panic!("finite guest calls must spend one cumulative allowance");
    }

    #[test]
    fn cache_reload_and_fresh_clone_keep_metering_with_independent_fuel() {
        let cache = FixtureDirectory::new();
        // Each call terminates on its own, so a regression that replenishes
        // fuel between exports fails an assertion instead of hanging.
        let body = format!(
            "(local $left i32) i32.const {} local.set $left
             (loop local.get $left i32.const 1 i32.sub local.tee $left br_if 0)
             i32.const 8",
            crate::host::runtime::INSTANCE_FUEL / 20,
        );
        let source = plugin_source(&body, "", 16);
        let binary = wasmer::wat2wasm(source.as_bytes()).unwrap().into_owned();
        let emulator: Arc<dyn CTVEmulator> = Arc::new(CTVAvailable);
        let mut original = WasmPluginHandle::<serde_json::Value>::new(
            cache.0.clone(),
            &emulator,
            SyncModuleLocator::Bytes(binary),
            bitcoin::Network::Regtest,
            None,
        )
        .unwrap();
        let initial = remaining_fuel(&mut original);
        assert!(initial > 0 && initial <= crate::host::runtime::INSTANCE_FUEL);
        assert_eq!(original.get_name().unwrap(), "ok");
        assert!(remaining_fuel(&mut original) < initial);
        exhaust_finite_calls(&mut original);

        let mut fresh = original.fresh_clone().unwrap();
        assert_eq!(remaining_fuel(&mut fresh), initial);
        assert_eq!(fresh.get_name().unwrap(), "ok");
        exhaust_finite_calls(&mut fresh);

        let mut reloaded = WasmPluginHandle::<serde_json::Value>::new(
            cache.0.clone(),
            &emulator,
            SyncModuleLocator::Key(original.id()),
            bitcoin::Network::Regtest,
            None,
        )
        .unwrap();
        assert_eq!(remaining_fuel(&mut reloaded), initial);
        assert_eq!(reloaded.get_name().unwrap(), "ok");
        exhaust_finite_calls(&mut reloaded);
    }

    #[tokio::test]
    async fn async_file_locator_accepts_wasm_and_rejects_oversized_or_nonfiles() {
        let directory = FixtureDirectory::new();
        let valid = directory.0.join("valid.wasm");
        let binary = wasmer::wat2wasm(b"(module)").unwrap().into_owned();
        std::fs::write(&valid, &binary).unwrap();
        match ModuleLocator::FileName(valid.to_str().unwrap().to_owned())
            .locate()
            .await
            .unwrap()
        {
            SyncModuleLocator::Bytes(bytes) => assert_eq!(bytes, binary),
            _ => panic!("file locator must return source bytes"),
        }
        let oversized = directory.0.join("oversized.wasm");
        std::fs::File::create(&oversized)
            .unwrap()
            .set_len(crate::host::wasm_cache::MAX_MODULE_BYTES as u64 + 1)
            .unwrap();
        for path in [&oversized, &directory.0] {
            assert!(ModuleLocator::FileName(path.to_str().unwrap().to_owned())
                .locate()
                .await
                .is_err());
        }
    }

    #[test]
    fn fresh_clones_preserve_indirect_host_calls_and_isolate_memory() {
        let mut original = plugin(
            "i32.const 0 i32.const 0 i32.const 32 i32.const 64 i32.const 0 \
             call_indirect (type $lookup_type) \
             i32.const 64 i32.load8_u i32.eqz if unreachable end i32.const 8",
            "(type $lookup_type (func (param i32 i32 i32 i32))) \
             (table 1 funcref) (elem (i32.const 0) $lookup)",
            8,
        );
        original.pass_string("changed").unwrap();
        assert_eq!(original.get_name().unwrap(), "changed");

        let mut cloned = original.fresh_clone().unwrap();
        assert_eq!(cloned.get_name().unwrap(), "ok");
        assert_eq!(original.get_name().unwrap(), "changed");
        drop(original);
        assert_eq!(cloned.get_name().unwrap(), "ok");
    }

    #[test]
    fn reads_valid_guest_strings_and_rejects_invalid_strings() {
        assert_eq!(plugin("i32.const 8", "", 16).get_name().unwrap(), "ok");
        for pointer in [0, -1, 65536] {
            assert!(plugin(&format!("i32.const {pointer}"), "", 16)
                .get_name()
                .is_err());
        }
        assert!(
            plugin("i32.const 65535", r#"(data (i32.const 65535) "x")"#, 16)
                .get_name()
                .is_err()
        );
        assert!(
            plugin("i32.const 32", r#"(data (i32.const 32) "\ff\00")"#, 16)
                .get_name()
                .is_err()
        );
    }

    #[test]
    fn checks_guest_allocations_before_writing() {
        let mut valid = plugin("i32.const 16", "", 16);
        valid.pass_string("hello").unwrap();
        assert_eq!(valid.get_name().unwrap(), "hello");
        for pointer in [0, -1, 65535, 65536] {
            assert!(plugin("i32.const 8", "", pointer)
                .pass_string("hello")
                .is_err());
        }
        assert!(valid.allocate(-1).is_err());
        assert!(valid.pass_string("embedded\0null").is_err());
    }

    #[test]
    fn malformed_host_calls_trap_instead_of_panicking_or_allocating() {
        for body in [
            "i32.const 8 i32.const -1 call $log i32.const 8",
            "i32.const 65536 i32.const 1 call $log i32.const 8",
            "i32.const 8 i32.const 16777217 call $log i32.const 8",
            "i32.const 8 i32.const 2 call $sign",
            "i32.const 8 i32.const -1 call $sign",
            "i32.const 0 i32.const 0 i32.const 65535 i32.const 64 call $lookup i32.const 8",
            "i32.const 0 i32.const 0 i32.const 32 i32.const 65536 call $lookup i32.const 8",
        ] {
            assert!(plugin(body, "", 16).get_name().is_err(), "{body}");
        }
    }

    #[test]
    fn host_calls_during_wasm_start_trap_without_initialized_memory() {
        let store = crate::host::runtime::new_store();
        let source = r#"(module
            (import "env" "sapio_v1_wasm_plugin_debug_log_string"
                (func $log (param i32 i32)))
            (func $start i32.const 0 i32.const 0 call $log)
            (start $start)
        )"#;
        let module = Module::new(&store, source.as_bytes()).unwrap();
        let emulator: Arc<dyn CTVEmulator> = Arc::new(CTVAvailable);
        let result = WasmPluginHandle::<serde_json::Value>::setup_plugin_inner(
            store,
            PathBuf::from("."),
            [0; 32],
            None,
            bitcoin::Network::Regtest,
            &emulator,
            module,
            WASMCacheID::generate(source.as_bytes()),
            InvocationBudget::new(),
        );
        assert!(result.is_err());
    }
}
