use super::wasm_cache;
use crate::CreateArgs;
use sapio::contract::Compiled;
use sapio_ctv_emulator_trait::NullEmulator;
use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use wasmer::{
    imports, Function, ImportObject, Instance, LazyInit, MemoryView, Module, NativeFunc, Store,
};
use wasmer_cache::Hash as WASMCacheID;

pub struct WasmPluginHandle {
    store: Store,
    env: super::HostEnvironment,
    import_object: ImportObject,
    module: Module,
    instance: Instance,
    key: wasmer_cache::Hash,
    net: bitcoin::Network,
}
use std::error::Error;
macro_rules! create_imports {
    ($store:ident, $env:ident $(,$names:ident)*) =>
    {
        imports! {

            "env" =>  {
                $( std::stringify!($names) => create_imports![$store $env $names], )*

            }
        }
    };

    [$store:ident $env:ident $name:ident] => {
        Function::new_native_with_env( &$store, $env.clone(), super::$name)
    };
}
impl WasmPluginHandle {
    pub fn id(&self) -> WASMCacheID {
        self.key
    }
    pub async fn new(
        emulator: NullEmulator,
        key: Option<&str>,
        file: Option<&OsStr>,
        net: bitcoin::Network,
        plugin_map: Option<HashMap<Vec<u8>, [u8; 32]>>,
    ) -> Result<Self, Box<dyn Error>> {
        // ensures that either key or file is passed
        key.xor(file.and(Some("")))
            .ok_or("Passed Both Key and File or Neither")?;
        let store = Store::default();
        let mut wasm_ctv_emulator = super::HostEnvironment {
            typ: "org".into(),
            org: "judica".into(),
            proj: "sapio-cli".into(),
            module_map: plugin_map.unwrap_or_else(HashMap::new).into(),
            store: Arc::new(Mutex::new(store.clone())),
            net,
            emulator: Arc::new(Mutex::new(emulator)),
            memory: LazyInit::new(),
            get_api: LazyInit::new(),
            forget: LazyInit::new(),
            create: LazyInit::new(),
            init: LazyInit::new(),
            allocate_wasm_bytes: LazyInit::new(),
        };

        let (module, key) = match (file, key) {
            (Some(file), _) => {
                let wasm_bytes = tokio::fs::read(file).await?;
                match wasm_cache::load_module("org", "judica", "sapio-cli", &store, &wasm_bytes) {
                    Ok(module) => module,
                    Err(_) => {
                        let store = Store::default();
                        let module = Module::new(&store, &wasm_bytes)?;
                        let key = wasm_cache::store_module(
                            "org",
                            "judica",
                            "sapio-cli",
                            &module,
                            &wasm_bytes,
                        )?;
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

    pub fn forget(&self, p: i32) -> Result<(), Box<dyn Error>> {
        Ok(self.env.forget_ref().ok_or("Uninitialized")?.call(p)?)
    }
    pub fn allocate(&self, len: i32) -> Result<i32, Box<dyn Error>> {
        Ok(self
            .env
            .allocate_wasm_bytes_ref()
            .ok_or("Uninitialized")?
            .call(len)?)
    }
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
    fn pass_string_inner(&self, s: &str, offset: i32) -> Result<(), Box<dyn Error>> {
        let memory = self.instance.exports.get_memory("memory")?;
        let mem: MemoryView<u8> = memory.view();
        for (idx, byte) in s.as_bytes().iter().enumerate() {
            mem[idx + offset as usize].set(*byte);
        }
        Ok(())
    }
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

pub trait PluginHandle {
    fn create(&self, c: &CreateArgs<String>) -> Result<Compiled, Box<dyn Error>>;
    fn get_api(&self) -> Result<serde_json::value::Value, Box<dyn Error>>;
}
impl PluginHandle for WasmPluginHandle {
    fn create(&self, c: &CreateArgs<String>) -> Result<Compiled, Box<dyn Error>> {
        let arg_str = serde_json::to_string_pretty(c)?;
        let offset = self.pass_string(&arg_str)?;
        let offset = self.env.create_ref().ok_or("Uninitialized")?.call(offset)?;
        let buf = self.read_to_vec(offset)?;
        self.forget(offset)?;
        let c: Result<String, String> = serde_json::from_slice(&buf)?;
        let v: Compiled = serde_json::from_str(&c?)?;
        Ok(v)
    }
    fn get_api(&self) -> Result<serde_json::value::Value, Box<dyn Error>> {
        let p = self.env.get_api_ref().ok_or("Uninitialized")?.call()?;
        let v = self.read_to_vec(p)?;
        self.forget(p)?;
        Ok(serde_json::from_slice(&v)?)
    }
}
