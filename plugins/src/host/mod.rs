use crate::CreateArgs;
use bitcoin::hashes::sha256;
use bitcoin::hashes::Hash;
use bitcoin::util::psbt::PartiallySignedTransaction;
use bitcoin::Amount;
pub use plugin_handle::PluginHandle;
pub use plugin_handle::WasmPluginHandle;
use sapio::contract::Compiled;
use sapio_ctv_emulator_trait::{CTVEmulator, NullEmulator};
use std::cell::Cell;
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};

use wasmer::*;

pub mod plugin_handle;
pub mod wasm_cache;

#[derive(WasmerEnv, Clone)]
pub struct HostEnvironmentInner {
    pub typ: String,
    pub org: String,
    pub proj: String,
    pub module_map: HashMap<Vec<u8>, [u8; 32]>,
    pub store: Arc<Mutex<Store>>,
    pub net: bitcoin::Network,
    pub emulator: Arc<Mutex<NullEmulator>>,
    #[wasmer(export)]
    pub memory: LazyInit<Memory>,
    #[wasmer(export(name = "sapio_v1_wasm_plugin_client_allocate_bytes"))]
    pub allocate_wasm_bytes: LazyInit<NativeFunc<i32, i32>>,
    #[wasmer(export(name = "sapio_v1_wasm_plugin_client_get_create_arguments"))]
    pub get_api: LazyInit<NativeFunc<(), i32>>,
    #[wasmer(export(name = "sapio_v1_wasm_plugin_client_get_name"))]
    pub get_name: LazyInit<NativeFunc<(), i32>>,
    #[wasmer(export(name = "sapio_v1_wasm_plugin_client_drop_allocation"))]
    pub forget: LazyInit<NativeFunc<i32, ()>>,
    #[wasmer(export(name = "sapio_v1_wasm_plugin_client_create"))]
    pub create: LazyInit<NativeFunc<i32, i32>>,
    #[wasmer(export(name = "sapio_v1_wasm_plugin_entry_point"))]
    pub init: LazyInit<NativeFunc<(), ()>>,
}

pub type HostEnvironment = Arc<Mutex<HostEnvironmentInner>>;

mod exports {
    use super::*;
    pub fn sapio_v1_wasm_plugin_lookup_module_name(
        env: &HostEnvironment,
        key: i32,
        len: i32,
        out: i32,
        ok: i32,
    ) {
        let env = env.lock().unwrap();
        let mut buf = vec![0u8; len as usize];
        for (src, dst) in env.memory_ref().unwrap().view()[key as usize..(key + len) as usize]
            .iter()
            .map(Cell::get)
            .zip(buf.iter_mut())
        {
            *dst = src;
        }
        env.memory_ref().unwrap().view::<u8>()[ok as usize].set(
            if let Some(b) = env.module_map.get(&buf) {
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
            },
        );
    }

    pub fn sapio_v1_wasm_plugin_create_contract(
        env: &HostEnvironment,
        key: i32,
        json: i32,
        json_len: i32,
        amt: u32,
    ) -> i32 {
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

        let mut v = vec![0u8; json_len as usize];
        for (src, dst) in env.memory_ref().unwrap().view()
            [json as usize..(json + json_len) as usize]
            .iter()
            .map(Cell::get)
            .zip(v.iter_mut())
        {
            *dst = src;
        }
        let emulator = env.emulator.lock().unwrap().clone();
        let mmap = env.module_map.clone();
        let typ = env.typ.clone();
        let org = env.org.clone();
        let proj = env.proj.clone();
        let net = env.net;
        let (tx, mut rx) = tokio::sync::oneshot::channel::<Compiled>();

        let handle = tokio::runtime::Handle::current();

        handle.spawn(async move {
            WasmPluginHandle::new(typ, org, proj, emulator, Some(&h), None, net, Some(mmap))
                .await
                .ok()
                .and_then(|sph| {
                    sph.create(&CreateArgs(
                        String::from_utf8_lossy(&v).to_owned().to_string(),
                        net,
                        Amount::from_sat(amt as u64),
                    ))
                    .ok()
                })
                .map(|comp| tx.send(comp))
        });

        tokio::task::block_in_place(|| loop {
            match rx.try_recv() {
                Ok(comp) => {
                    return (move || -> Result<i32, Box<dyn std::error::Error>> {
                        let comp_s = serde_json::to_string(&comp)?;
                        let bytes: i32 = env
                            .allocate_wasm_bytes_ref()
                            .unwrap()
                            .call(comp_s.len() as i32)?;
                        for (byte, c) in env.memory_ref().unwrap().view::<u8>()[bytes as usize..]
                            .iter()
                            .zip(comp_s.as_bytes())
                        {
                            byte.set(*c);
                        }
                        Ok(bytes)
                    })()
                    .unwrap_or(0);
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => return 0,
                _ => (),
            };
        })
    }

    pub fn sapio_v1_wasm_plugin_debug_log_string(env: &HostEnvironment, a: i32, len: i32) {
        let env = env.lock().unwrap();
        let stdout = std::io::stdout();
        let lock = stdout.lock();
        let mut w = std::io::BufWriter::new(lock);
        let mem = env.memory_ref().unwrap().view::<u8>();
        for byte in mem[a as usize..(a + len) as usize].iter().map(Cell::get) {
            w.write(&[byte]).unwrap();
        }
        w.write("\n".as_bytes()).unwrap();
    }
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
        let clause = env.emulator.lock().unwrap().get_signer_for(h).unwrap();
        let s = serde_json::to_string_pretty(&clause).unwrap();
        let bytes = env
            .allocate_wasm_bytes_ref()
            .unwrap()
            .call(s.len() as i32)
            .unwrap();
        for (byte, c) in env.memory_ref().unwrap().view::<u8>()[bytes as usize..]
            .iter()
            .zip(s.as_bytes())
        {
            byte.set(*c);
        }
        bytes
    }

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
        let psbt: PartiallySignedTransaction = serde_json::from_slice(&buf[..]).unwrap();
        let psbt = env.emulator.lock().unwrap().sign(psbt).unwrap();
        buf.clear();
        let s = serde_json::to_string_pretty(&psbt).unwrap();
        let bytes = env
            .allocate_wasm_bytes_ref()
            .unwrap()
            .call(s.len() as i32)
            .unwrap();
        for (byte, c) in env.memory_ref().unwrap().view::<u8>()[bytes as usize..]
            .iter()
            .zip(s.as_bytes())
        {
            byte.set(*c);
        }
        bytes
    }
}
