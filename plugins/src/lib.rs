use sapio::contract::{Compilable, Context};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::Arc;

fn json_wrapped_string<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: for<'t> Deserialize<'t>,
{
    let s = String::deserialize(d)?;
    serde_json::from_str(&s).map_err(serde::de::Error::custom)
}
#[derive(Serialize, Deserialize)]
pub struct CreateArgs<S: for<'t> Deserialize<'t>>(
    /// We use json_wrapped_string to encode S to allow for a client to pass in
    /// CreateArgs without knowing the underlying type S.
    #[serde(deserialize_with = "json_wrapped_string")]
    pub S,
    pub bitcoin::Network,
    #[serde(with = "bitcoin::util::amount::serde::as_sat")] pub bitcoin::util::amount::Amount,
);

#[derive(Serialize, Deserialize)]
enum PluginError {
    EncodingError,
}

pub trait Plugin: JsonSchema + Sized {
    fn get_api_inner() -> *mut c_char {
        encode_json(&schemars::schema_for!(Self))
    }

    fn get_api() -> *mut c_char;

    unsafe fn create(c: *mut c_char) -> *mut c_char;
}

fn encode_json<S: Serialize>(s: &S) -> *mut c_char {
    if let Ok(Ok(c)) = serde_json::to_string_pretty(s).map(CString::new) {
        c.into_raw()
    } else {
        0 as *mut c_char
    }
}

#[cfg(feature = "host")]
pub mod host {

    use bitcoin::hashes::sha256;
    use bitcoin::hashes::Hash;
    use bitcoin::util::psbt::PartiallySignedTransaction;
    use sapio_ctv_emulator_trait::{CTVEmulator, NullEmulator};

    use std::cell::Cell;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use wasmer::*;

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
        }
        bytes
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
}

#[cfg(feature = "client")]
pub mod client {
    use super::*;
    use bitcoin::hashes::Hash;
    use sapio_ctv_emulator_trait::CTVEmulator;
    use std::error::Error;
    extern "C" {
        fn wasm_emulator_sign(psbt: i32, len: u32) -> i32;
        fn wasm_emulator_signer_for(hash: i32) -> i32;
        fn host_log(a: i32, len: i32);
    }

    pub fn log(s: &str) {
        unsafe {
            host_log(s.as_ptr() as i32, s.len() as i32);
        }
    }

    pub struct WasmHostEmulator;
    impl CTVEmulator for WasmHostEmulator {
        fn get_signer_for(
            &self,
            h: bitcoin::hashes::sha256::Hash,
        ) -> std::result::Result<
            miniscript::policy::concrete::Policy<bitcoin::PublicKey>,
            sapio_ctv_emulator_trait::EmulatorError,
        > {
            let mut inner = h.into_inner();
            let signer = unsafe { wasm_emulator_signer_for(&mut inner[0] as *mut u8 as i32) };
            let signer = unsafe { CString::from_raw(signer as *mut c_char) };
            Ok(serde_json::from_slice(signer.to_bytes()).unwrap())
        }
        fn sign(
            &self,
            psbt: bitcoin::util::psbt::PartiallySignedTransaction,
        ) -> std::result::Result<
            bitcoin::util::psbt::PartiallySignedTransaction,
            sapio_ctv_emulator_trait::EmulatorError,
        > {
            let s = serde_json::to_string_pretty(&psbt).unwrap();
            let len = s.len();
            let ret = unsafe {
                CString::from_raw(wasm_emulator_sign(s.as_ptr() as i32, len as u32) as *mut c_char)
            };
            let j = serde_json::from_slice(ret.as_bytes()).unwrap();
            Ok(j)
        }
    }

    // T
    #[no_mangle]
    unsafe fn forget_allocated_wasm_bytes(s: *mut c_char) {
        CString::from_raw(s);
    }
    #[no_mangle]
    fn allocate_wasm_bytes(len: u32) -> *mut c_char {
        CString::new(vec![1; len as usize]).unwrap().into_raw()
    }

    /// Defined here for convenient binding
    pub unsafe fn create<T>(c: *mut c_char) -> *mut c_char
    where
        T: Serialize + for<'a> Deserialize<'a> + Compilable + 'static,
    {
        let res = create_result_err::<T>(c);
        encode_json(&res)
    }

    pub unsafe fn create_result_err<T>(c: *mut c_char) -> Result<String, String>
    where
        T: Serialize + for<'a> Deserialize<'a> + Compilable + 'static,
    {
        create_result::<T>(c).map_err(|e| e.to_string())
    }
    pub unsafe fn create_result<T>(c: *mut c_char) -> Result<String, Box<dyn Error>>
    where
        T: Serialize + for<'a> Deserialize<'a> + Compilable + 'static,
    {
        let s = CString::from_raw(c);
        let CreateArgs::<T>(s, net, amt) = serde_json::from_slice(s.to_bytes())?;
        let ctx = Context::new(net, amt, Some(Arc::new(client::WasmHostEmulator)));
        Ok(serde_json::to_string_pretty(&s.compile(&ctx)?)?)
    }
}
