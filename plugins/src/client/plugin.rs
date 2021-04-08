// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! binding for making a type into a plugin
use super::*;
/// The `Plugin` trait is used to provide bindings for a WASM Plugin.
/// It's not intended to be used internally, just as bindings.
pub trait Plugin: JsonSchema + Sized + for<'a> Deserialize<'a> + Compilable {
    /// gets the jsonschema for the plugin type, which is the API for calling create.
    fn get_api_inner() -> *mut c_char {
        encode_json(&schemars::schema_for!(Self))
    }

    /// creates an instance of the plugin from a json pointer and outputs a result pointer
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
        let ctx = Context::new(net, amt, Arc::new(client::WasmHostEmulator));
        Ok(serde_json::to_string_pretty(&s.compile(&ctx)?)?)
    }
    /// binds this type to the wasm interface, must be called before the plugin can be used.
    unsafe fn register(name: &'static str) {
        sapio_v1_wasm_plugin_client_get_create_arguments_ptr = Self::get_api_inner;
        sapio_v1_wasm_plugin_client_create_ptr = Self::create;
        sapio_plugin_name = name;
    }
}

/// Helper function for encoding a JSON into WASM linear memory
fn encode_json<S: Serialize>(s: &S) -> *mut c_char {
    if let Ok(Ok(c)) = serde_json::to_string_pretty(s).map(CString::new) {
        c.into_raw()
    } else {
        0 as *mut c_char
    }
}

/// A helper macro to implement the plugin interface for a plugin-type
/// and register it to the plugin entry point.
///
/// U.B. to call REGISTER more than once because of the internal #[no_mangle]
#[macro_export]
macro_rules! REGISTER {
    [$plugin:ident] => {
        impl Plugin for $plugin {
        }
        #[no_mangle]
        unsafe fn sapio_v1_wasm_plugin_entry_point() {
            $plugin::register(stringify!($plugin));
        }
    };
}
