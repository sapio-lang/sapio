
use bitcoin::hashes::sha256;
use bitcoin::hashes::Hash;
use bitcoin::util::psbt::PartiallySignedTransaction;
use sapio_ctv_emulator_trait::{CTVEmulator, NullEmulator};

use std::cell::Cell;
use std::io::Write;
use std::sync::{Arc, Mutex};
use wasmer::*;

pub mod plugin_handle;
pub use plugin_handle::SapioPluginHandle;
pub mod wasm_cache;

#[derive(WasmerEnv, Clone)]
pub struct EmulatorEnv {
    pub emulator: Arc<Mutex<NullEmulator>>,
    #[wasmer(export)]
    pub memory: LazyInit<Memory>,
    #[wasmer(export)]
    pub allocate_wasm_bytes: LazyInit<NativeFunc<i32, i32>>,
}

pub fn host_log(env: &EmulatorEnv, a: i32, len: i32) {
    let stdout = std::io::stdout();
    let lock = stdout.lock();
    let mut w = std::io::BufWriter::new(lock);
    let mem = env.memory_ref().unwrap().view::<u8>();
    for byte in mem[a as usize..(a + len) as usize].iter().map(Cell::get) {
        w.write(&[byte]).unwrap();
    }
    w.write("\n".as_bytes()).unwrap();
}
pub fn wasm_emulator_signer_for(env: &EmulatorEnv, hash: i32) -> i32 {
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
    }    bytes
}

pub fn wasm_emulator_sign(env: &EmulatorEnv, psbt: i32, len: u32) -> i32 {
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
