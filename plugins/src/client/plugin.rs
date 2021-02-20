
use super::*;
pub trait Plugin: JsonSchema + Sized + for<'a> Deserialize<'a> + Compilable {
    fn get_api_inner() -> *mut c_char {
        encode_json(&schemars::schema_for!(Self))
    }

    unsafe fn create(c: *mut c_char) -> *mut c_char {
        let res = Self::create_result_err(c);
        encode_json(&res)
    }

    unsafe fn create_result_err(c: *mut c_char) -> Result<String, String> {
        Self::create_result(c).map_err(|e| e.to_string())
    }
    unsafe fn create_result(c: *mut c_char) -> Result<String, Box<dyn Error>> {
        let s = CString::from_raw(c);
        let CreateArgs::<Self>(s, net, amt) = serde_json::from_slice(s.to_bytes())?;
        let ctx = Context::new(net, amt, Some(Arc::new(client::WasmHostEmulator)));
        Ok(serde_json::to_string_pretty(&s.compile(&ctx)?)?)
    }

    unsafe fn register() {
        sapio_v1_wasm_plugin_client_get_create_arguments_ptr = Self::get_api_inner;
        sapio_v1_wasm_plugin_client_create_ptr = Self::create;
    }
}

fn encode_json<S: Serialize>(s: &S) -> *mut c_char {
    if let Ok(Ok(c)) = serde_json::to_string_pretty(s).map(CString::new) {
        c.into_raw()
    } else {
        0 as *mut c_char
    }
}

#[macro_export]
macro_rules! REGISTER {
    [$plugin:ident] => {
        impl Plugin for $plugin {
        }
        #[no_mangle]
        unsafe fn sapio_v1_wasm_plugin_entry_point() {
            $plugin::register();
        }
    };
}