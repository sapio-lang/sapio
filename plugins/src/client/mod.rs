    use super::*;
    use bitcoin::hashes::Hash;
    use bitcoin::Amount;
    use sapio::contract::Compiled;
    use sapio_ctv_emulator_trait::CTVEmulator;
    use serde_json::Value;
    use std::error::Error;
    extern "C" {
        fn wasm_emulator_sign(psbt: i32, len: u32) -> i32;
        fn wasm_emulator_signer_for(hash: i32) -> i32;
        fn host_log(a: i32, len: i32);
        fn host_remote_call(key: i32, json: i32, json_len: i32, amt: u32) -> i32;
        fn host_lookup_module_name(key: i32, len: i32, out: i32, ok: i32);
    }

    pub fn log(s: &str) {
        unsafe {
            host_log(s.as_ptr() as i32, s.len() as i32);
        }
    }

    pub fn remote_call_by_key(key: &[u8; 32], args: Value, amt: Amount) -> Option<Compiled> {
        unsafe {
            let s = args.to_string();
            let l = s.len();
            let p = host_remote_call(
                key.as_ptr() as i32,
                s.as_ptr() as i32,
                l as i32,
                amt.as_sat() as u32,
            );
            if p != 0 {
                let cs = CString::from_raw(p as *mut c_char);
                serde_json::from_slice(cs.as_bytes()).ok()
            } else {
                None
            }
        }
    }

    pub fn lookup_module_name(key: &str) -> Option<[u8; 32]> {
        unsafe {
            let mut res = [0u8; 32];
            let mut ok = 0u8;
            host_lookup_module_name(
                key.as_ptr() as i32,
                key.len() as i32,
                &mut res as *mut [u8; 32] as i32,
                &mut ok as *mut u8 as i32,
            );
            if ok == 0 {
                None
            } else {
                Some(res)
            }
        }
    }

    pub fn remote_call(key: &str, args: Value, amt: Amount) -> Option<Compiled> {
        let key = lookup_module_name(key)?;
        remote_call_by_key(&key, args, amt)
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