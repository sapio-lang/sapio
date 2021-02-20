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

    fn sapio_v1_wasm_plugin_client_get_create_arguments() -> *mut c_char;

    unsafe fn sapio_v1_wasm_plugin_client_create(c: *mut c_char) -> *mut c_char;
}

fn encode_json<S: Serialize>(s: &S) -> *mut c_char {
    if let Ok(Ok(c)) = serde_json::to_string_pretty(s).map(CString::new) {
        c.into_raw()
    } else {
        0 as *mut c_char
    }
}

#[cfg(feature = "host")]
pub mod host;

#[cfg(feature = "client")]
pub mod client;
