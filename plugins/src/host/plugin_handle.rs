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

pub struct SapioPluginHandle {
    store: Store,
    env: super::EmulatorEnv,
    import_object: ImportObject,
    module: Module,
    instance: Instance,
    /// Functions
    get_api: NativeFunc<(), i32>,
    forget: NativeFunc<i32, ()>,
    allocate: NativeFunc<i32, i32>,
    create: NativeFunc<i32, i32>,
    key: wasmer_cache::Hash,
    net: bitcoin::Network,
}
use std::error::Error;
impl SapioPluginHandle {
    pub fn id(&self) -> WASMCacheID {
        self.key
    }
    pub async fn new(
        emulator: NullEmulator,
        key: Option<&str>,
        file: Option<&OsStr>,
        net: bitcoin::Network,
    ) -> Result<Self, Box<dyn Error>> {
        // ensures that either key or file is passed
        key.xor(file.and(Some("")))
            .ok_or("Passed Both Key and File or Neither")?;
        let store = Store::default();
        let wasm_ctv_emulator = super::EmulatorEnv {
            typ: "org".into(),
            org: "judica".into(),
            proj: "sapio-cli".into(),
            module_map: HashMap::new(),
            store: Arc::new(Mutex::new(store.clone())),
            net,
            emulator: Arc::new(Mutex::new(emulator)),
            memory: LazyInit::new(),
            allocate_wasm_bytes: LazyInit::new(),
        };
        let f = Function::new_native_with_env(
            &store,
            wasm_ctv_emulator.clone(),
            super::wasm_emulator_signer_for,
        );
        let g = Function::new_native_with_env(
            &store,
            wasm_ctv_emulator.clone(),
            super::wasm_emulator_sign,
        );
        let log = Function::new_native_with_env(&store, wasm_ctv_emulator.clone(), super::host_log);
        let remote_call =
            Function::new_native_with_env(&store, wasm_ctv_emulator.clone(), super::remote_call);
        let lookup_module_name = Function::new_native_with_env(
            &store,
            wasm_ctv_emulator.clone(),
            super::host_lookup_module_name,
        );

        let import_object = imports! {
            "env" => {
                "wasm_emulator_signer_for" => f,
                "wasm_emulator_sign" => g,
                "host_log" => log,
                "host_remote_call" => remote_call,
                "host_lookup_module_name" => lookup_module_name,
            }
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
        let instance = Instance::new(&module, &import_object)?;

        let get_api = instance
            .exports
            .get_function("get_api")?
            .native::<(), i32>()?;
        let forget = instance
            .exports
            .get_function("forget_allocated_wasm_bytes")?
            .native::<i32, ()>()?;
        let allocate = instance
            .exports
            .get_function("allocate_wasm_bytes")?
            .native::<i32, i32>()?;
        let create = instance
            .exports
            .get_function("create")?
            .native::<i32, i32>()?;
        Ok(SapioPluginHandle {
            store,
            env: wasm_ctv_emulator,
            net,
            import_object,
            module,
            instance,
            get_api,
            forget,
            allocate,
            create,
            key,
        })
    }

    pub fn get_api(&self) -> Result<serde_json::value::Value, Box<dyn Error>> {
        let p = self.get_api.call()?;
        let v = self.read_to_vec(p)?;
        self.forget(p)?;
        Ok(serde_json::from_slice(&v)?)
    }
    pub fn forget(&self, p: i32) -> Result<(), Box<dyn Error>> {
        Ok(self.forget.call(p)?)
    }
    pub fn allocate(&self, len: i32) -> Result<i32, Box<dyn Error>> {
        Ok(self.allocate.call(len)?)
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
    pub fn create(&self, c: &CreateArgs<String>) -> Result<Compiled, Box<dyn Error>> {
        let arg_str = serde_json::to_string_pretty(c)?;
        let offset = self.pass_string(&arg_str)?;
        let offset = self.create.call(offset)?;
        let buf = self.read_to_vec(offset)?;
        self.forget(offset)?;
        let c: Result<String, String> = serde_json::from_slice(&buf)?;
        let v: Compiled = serde_json::from_str(&c?)?;
        Ok(v)
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
