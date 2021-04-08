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
use sapio_ctv_emulator_trait::CTVEmulator;
use std::error::Error;
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
    pub async fn load_all_keys(
        typ: String,
        org: String,
        proj: String,
        emulator: NullEmulator,
        net: bitcoin::Network,
        plugin_map: Option<HashMap<Vec<u8>, [u8; 32]>>,
    ) -> Result<Vec<Self>, Box<dyn Error>> {
        let mut r = vec![];
        for key in get_all_keys_from_fs(&typ, &org, &proj)? {
            let wph = Self::new(
                typ.clone(),
                org.clone(),
                proj.clone(),
                &emulator,
                Some(&key),
                None,
                net,
                plugin_map.clone(),
            )
            .await?;
            r.push(wph)
        }
        Ok(r)
    }

    /// Create an plugin handle. Only one of key or file should be set, and one
    /// should be set.
    pub async fn new(
        typ: String,
        org: String,
        proj: String,
        emulator: &Arc<dyn CTVEmulator>,
        key: Option<&str>,
        file: Option<&OsStr>,
        net: bitcoin::Network,
        plugin_map: Option<HashMap<Vec<u8>, [u8; 32]>>,
    ) -> Result<Self, Box<dyn Error>> {
        // ensures that either key or file is passed
        key.xor(file.and(Some("")))
            .ok_or("Passed Both Key and File or Neither")?;
        let store = Store::default();

        let (module, key) = match (file, key) {
            (Some(file), _) => {
                let wasm_bytes = tokio::fs::read(file).await?;
                match wasm_cache::load_module(&typ, &org, &proj, &store, &wasm_bytes) {
                    Ok(module) => module,
                    Err(_) => {
                        let store = Store::default();
                        let module = Module::new(&store, &wasm_bytes)?;
                        let key =
                            wasm_cache::store_module(&typ, &org, &proj, &module, &wasm_bytes)?;
                        (module, key)
                    }
                }
            }
            (_, Some(key)) => {
                let key = WASMCacheID::from_str(key)?;
                wasm_cache::load_module_key("org", "judica", "sapio-cli", &store, key)?
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

        let mut wasm_ctv_emulator = Arc::new(Mutex::new(HostEnvironmentInner {
            typ,
            org,
            proj,
            module_map: plugin_map.unwrap_or_else(HashMap::new).into(),
            store: Arc::new(Mutex::new(store.clone())),
            net,
            emulator: emulator.clone(),
            memory: LazyInit::new(),
            get_api: LazyInit::new(),
            get_name: LazyInit::new(),
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
    pub fn forget(&self, p: i32) -> Result<(), Box<dyn Error>> {
        Ok(self
            .env
            .lock()
            .unwrap()
            .forget_ref()
            .ok_or("Uninitialized")?
            .call(p)?)
    }

    /// create an allocation
    pub fn allocate(&self, len: i32) -> Result<i32, Box<dyn Error>> {
        Ok(self
            .env
            .lock()
            .unwrap()
            .allocate_wasm_bytes_ref()
            .ok_or("Uninitialized")?
            .call(len)?)
    }

    /// pass a string to the WASM plugin
    pub fn pass_string(&self, s: &str) -> Result<i32, Box<dyn Error>> {
        let offset = self.allocate(s.len() as i32)?;
        match self.pass_string_inner(s, offset) {
            Ok(_) => Ok(offset),
            e @ Err(_) => {
                self.forget(offset)?;
                return e.map(|_| 0);
            }
        }
    }

    /// helper for string passing
    fn pass_string_inner(&self, s: &str, offset: i32) -> Result<(), Box<dyn Error>> {
        let memory = self.instance.exports.get_memory("memory")?;
        let mem: MemoryView<u8> = memory.view();
        for (idx, byte) in s.as_bytes().iter().enumerate() {
            mem[idx + offset as usize].set(*byte);
        }
        Ok(())
    }

    /// read something from wasm memory, null terminated
    fn read_to_vec(&self, p: i32) -> Result<Vec<u8>, Box<dyn Error>> {
        let memory = self.instance.exports.get_memory("memory")?;
        let mem: MemoryView<u8> = memory.view();
        Ok(mem[p as usize..]
            .iter()
            .map(Cell::get)
            .take_while(|i| *i != 0)
            .collect())
    }
}

impl PluginHandle for WasmPluginHandle {
    fn create(&self, c: &CreateArgs<String>) -> Result<Compiled, Box<dyn Error>> {
        let arg_str = serde_json::to_string_pretty(c)?;
        let offset = self.pass_string(&arg_str)?;
        let create_func = {
            let env = self.env.lock().unwrap();
            env.create.clone()
        };
        let offset = create_func.get_ref().ok_or("Uninitialized")?.call(offset)?;
        let buf = self.read_to_vec(offset)?;
        self.forget(offset)?;
        let c: Result<String, String> = serde_json::from_slice(&buf)?;
        let v: Compiled = serde_json::from_str(&c?)?;
        Ok(v)
    }
    fn get_api(&self) -> Result<serde_json::value::Value, Box<dyn Error>> {
        let p = self
            .env
            .lock()
            .unwrap()
            .get_api_ref()
            .ok_or("Uninitialized")?
            .call()?;
        let v = self.read_to_vec(p)?;
        self.forget(p)?;
        Ok(serde_json::from_slice(&v)?)
    }
    fn get_name(&self) -> Result<String, Box<dyn Error>> {
        let p = self
            .env
            .lock()
            .unwrap()
            .get_name_ref()
            .ok_or("Uninitialized")?
            .call()?;
        let v = self.read_to_vec(p)?;
        self.forget(p)?;
        Ok(String::from_utf8_lossy(&v).to_string())
    }
}
