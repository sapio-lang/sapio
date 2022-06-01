// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//!  a plugin handle for a wasm plugin.
use super::*;
use crate::host::exports::*;
use crate::host::wasm_cache::get_all_keys_from_fs;
use crate::host::{HostEnvironment, HostEnvironmentInner};
use crate::plugin_handle::PluginHandle;
use crate::API;
use sapio::contract::CompilationError;
use sapio_base::effects::EffectPath;
use sapio_ctv_emulator_trait::CTVEmulator;
use std::error::Error;
use std::path::PathBuf;
use wasmer::Memory;
pub struct WasmPluginHandle {
    store: Store,
    env: HostEnvironment,
    import_object: ImportObject,
    module: Module,
    instance: Instance,
    key: wasmer_cache::Hash,
    net: bitcoin::Network,
}
impl WasmPluginHandle {
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
                Some(&key),
                None,
                net,
                plugin_map.clone(),
            )?;
            r.push(wph)
        }
        Ok(r)
    }

    pub async fn new_async<I: Into<PathBuf> + Clone>(
        path: I,
        emulator: &Arc<dyn CTVEmulator>,
        key: Option<&str>,
        file: Option<&OsStr>,
        net: bitcoin::Network,
        plugin_map: Option<BTreeMap<Vec<u8>, [u8; 32]>>,
    ) -> Result<Self, Box<dyn Error>> {
        let file = if let Some(f) = file {
            Some(tokio::fs::read(f).await?)
        } else {
            None
        };
        Self::new(path, emulator, key, file.as_ref(), net, plugin_map)
    }
    /// Create an plugin handle. Only one of key or file should be set, and one
    /// should be set.
    /// TODO: Revert to async?
    pub fn new<I: Into<PathBuf> + Clone>(
        path: I,
        emulator: &Arc<dyn CTVEmulator>,
        key: Option<&str>,
        file: Option<&Vec<u8>>,
        net: bitcoin::Network,
        plugin_map: Option<BTreeMap<Vec<u8>, [u8; 32]>>,
    ) -> Result<Self, Box<dyn Error>> {
        // ensures that either key or file is passed
        key.xor(file.and(Some("")))
            .ok_or("Passed Both Key and File or Neither")?;
        let store = Store::default();

        let (module, key) = match (file, key) {
            (Some(wasm_bytes), _) => {
                match wasm_cache::load_module(path.clone(), &store, &wasm_bytes) {
                    Ok(module) => module,
                    Err(_) => {
                        let store = Store::default();
                        let module = Module::new(&store, &wasm_bytes)?;
                        let key = wasm_cache::store_module(path.clone(), &module, &wasm_bytes)?;
                        (module, key)
                    }
                }
            }
            (_, Some(key)) => {
                let key = WASMCacheID::from_str(key)?;
                wasm_cache::load_module_key(path.clone(), &store, key)?
            }
            _ => unreachable!(),
        };

        macro_rules! create_imports {
            ($store:ident, $env:ident $(,$names:ident)*) =>
            {
                imports! {
                    "env" =>  {
                        $( std::stringify!($names) => Function::new_native_with_env( &$store, $env.clone(), $names) ,)*
                    }
                }
            };
        }
        let mut this = [0; 32];
        this.clone_from_slice(&hex::decode(key.to_string())?);
        let mut wasm_ctv_emulator = Arc::new(Mutex::new(HostEnvironmentInner {
            path: path.into().clone(),
            this,
            module_map: plugin_map.unwrap_or_else(BTreeMap::new).into(),
            store: Arc::new(Mutex::new(store.clone())),
            net,
            emulator: emulator.clone(),
            memory: LazyInit::new(),
            get_api: LazyInit::new(),
            get_name: LazyInit::new(),
            get_logo: LazyInit::new(),
            forget: LazyInit::new(),
            create: LazyInit::new(),
            init: LazyInit::new(),
            allocate_wasm_bytes: LazyInit::new(),
        }));
        let import_object = create_imports!(
            store,
            wasm_ctv_emulator,
            sapio_v1_wasm_plugin_ctv_emulator_signer_for,
            sapio_v1_wasm_plugin_ctv_emulator_sign,
            sapio_v1_wasm_plugin_debug_log_string,
            sapio_v1_wasm_plugin_create_contract,
            sapio_v1_wasm_plugin_get_api,
            sapio_v1_wasm_plugin_get_name,
            sapio_v1_wasm_plugin_get_logo,
            sapio_v1_wasm_plugin_lookup_module_name
        );

        let instance = Instance::new(&module, &import_object)?;
        use wasmer::WasmerEnv;
        wasm_ctv_emulator.init_with_instance(&instance)?;

        wasm_ctv_emulator
            .lock()
            .unwrap()
            .init_ref()
            .ok_or("No Init Function Specified")?
            .call()?;

        Ok(WasmPluginHandle {
            store,
            env: wasm_ctv_emulator,
            net,
            import_object,
            module,
            instance,
            key,
        })
    }

    /// forget an allocated pointer
    pub fn forget(&self, p: i32) -> Result<(), CompilationError> {
        Ok(self
            .env
            .lock()
            .unwrap()
            .forget_ref()
            .ok_or_else(|| CompilationError::ModuleCouldNotFindFunction("forget".into()))?
            .call(p)
            .map_err(|e| CompilationError::ModuleCouldNotDeallocate(p, e.into()))?)
    }

    /// create an allocation
    pub fn allocate(&self, len: i32) -> Result<i32, CompilationError> {
        self.env
            .lock()
            .unwrap()
            .allocate_wasm_bytes_ref()
            .ok_or_else(|| {
                CompilationError::ModuleCouldNotFindFunction("allocate_wasm_bytes".into())
            })?
            .call(len)
            .map_err(|e| CompilationError::ModuleCouldNotAllocateError(len, e.into()))
    }

    /// pass a string to the WASM plugin
    pub fn pass_string(&self, s: &str) -> Result<i32, CompilationError> {
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
        let memory = self.get_memory()?;
        let mem: MemoryView<'_, u8> = memory.view();
        for (idx, byte) in s.as_bytes().iter().enumerate() {
            mem[idx + offset as usize].set(*byte);
        }
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
        let memory = self.get_memory()?;
        let mem: MemoryView<'_, u8> = memory.view();
        Ok(mem[p as usize..]
            .iter()
            .map(Cell::get)
            .take_while(|i| *i != 0)
            .collect())
    }
}

impl PluginHandle for WasmPluginHandle {
    type Input = CreateArgs<serde_json::Value>;
    type Output = Compiled;
    fn call(&self, path: &EffectPath, c: &Self::Input) -> Result<Self::Output, CompilationError> {
        let arg_str = serde_json::to_string(c).map_err(CompilationError::SerializationError)?;
        let args_ptr = self.pass_string(&arg_str)?;
        let path_str = serde_json::to_string(path).map_err(CompilationError::SerializationError)?;
        let path_ptr = self.pass_string(&path_str)?;
        let create_func = {
            let env = self.env.lock().unwrap();
            env.create.clone()
        };
        let result_ptr = create_func
            .get_ref()
            .ok_or_else(|| CompilationError::ModuleCouldNotFindFunction("create".into()))?
            .call(path_ptr, args_ptr)
            .map_err(|e| {
                CompilationError::ModuleCouldNotCreateContract(path.clone(), c.clone(), e.into())
            })?;
        let buf = self.read_to_vec(result_ptr)?;
        self.forget(result_ptr)?;
        let v: Result<Self::Output, String> =
            serde_json::from_slice(&buf).map_err(CompilationError::DeserializationError)?;
        v.map_err(CompilationError::ModuleCompilationErrorUnsendable)
    }
    fn get_api(&self) -> Result<API<Self::Input, Self::Output>, CompilationError> {
        let p = self
            .env
            .lock()
            .unwrap()
            .get_api_ref()
            .ok_or_else(|| CompilationError::ModuleCouldNotFindFunction("get_api".into()))?
            .call()
            .map_err(|e| CompilationError::ModuleCouldNotGetAPI(e.into()))?;
        let v = self.read_to_vec(p)?;
        self.forget(p)?;
        Ok(serde_json::from_slice(&v).map_err(CompilationError::DeserializationError)?)
    }
    fn get_name(&self) -> Result<String, CompilationError> {
        let p = self
            .env
            .lock()
            .unwrap()
            .get_name_ref()
            .ok_or_else(|| CompilationError::ModuleCouldNotFindFunction("get_name".into()))?
            .call()
            .map_err(|e| CompilationError::ModuleCouldNotGetName(e.into()))?;
        let v = self.read_to_vec(p)?;
        self.forget(p)?;
        Ok(String::from_utf8_lossy(&v).to_string())
    }

    fn get_logo(&self) -> Result<String, CompilationError> {
        let p = self
            .env
            .lock()
            .unwrap()
            .get_logo_ref()
            .ok_or_else(|| CompilationError::ModuleCouldNotFindFunction("get_logo".into()))?
            .call()
            .map_err(|e| CompilationError::ModuleCouldNotGetLogo(e.into()))?;
        let v = self.read_to_vec(p)?;
        self.forget(p)?;
        Ok(String::from_utf8_lossy(&v).to_string())
    }
}
