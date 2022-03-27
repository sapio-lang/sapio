// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! binding for making a type into a plugin
use super::*;
use sapio_base::effects::EffectPath;
use sapio_base::effects::PathFragment;
use sapio_base::serialization_helpers::SArc;

use std::convert::TryFrom;
/// The `Plugin` trait is used to provide bindings for a WASM Plugin.
/// It's not intended to be used internally, just as bindings.
pub trait Plugin: JsonSchema + Sized + for<'a> Deserialize<'a>
where
    <<Self as client::plugin::Plugin>::ToType as std::convert::TryFrom<Self>>::Error:
        std::error::Error + 'static,
{
    type ToType: Compilable + TryFrom<Self>;
    /// gets the jsonschema for the plugin type, which is the API for calling create.
    fn get_api_inner() -> *mut c_char {
        encode_json(&schemars::schema_for!(CreateArgs::<Self>))
    }

    /// creates an instance of the plugin from a json pointer and outputs a result pointer
    unsafe fn create(p: *mut c_char, c: *mut c_char) -> *mut c_char {
        let res = Self::create_result_err(p, c);
        encode_json(&res)
    }

    unsafe fn create_result_err(p: *mut c_char, c: *mut c_char) -> Result<String, String> {
        Self::create_result(p, c).map_err(|e| e.to_string())
    }
    unsafe fn create_result(p: *mut c_char, c: *mut c_char) -> Result<String, Box<dyn Error>> {
        let s = CString::from_raw(c);
        let path = CString::from_raw(p);
        let CreateArgs::<Self> {
            arguments,
            context:
                ContextualArguments {
                    network,
                    amount,
                    effects,
                },
        } = serde_json::from_slice(s.to_bytes())?;
        // TODO: In theory, these trampoline bounds are robust/serialization safe...
        // But the API needs stiching to the parent in a sane way...
        let caller = lookup_this_module_name()
            .map(|s| bitcoin::hashes::hex::ToHex::to_hex(&s[..]))
            .ok_or("Host Error: Should always be able to identify module's own ID")?;
        let path: EffectPath = EffectPath::push_owned(
            Some(EffectPath::push(
                Some(Arc::new(serde_json::from_str(path.to_str()?)?)),
                PathFragment::Root,
            )),
            PathFragment::Named(SArc(Arc::new(caller.into()))),
        );

        let ctx = Context::new(
            network,
            amount,
            Arc::new(client::WasmHostEmulator),
            path,
            // TODO: load database?
            Arc::new(effects),
        );
        let converted = Self::ToType::try_from(arguments)?;
        let compiled = converted.compile(ctx)?;
        Ok(serde_json::to_string(&compiled)?)
    }
    /// binds this type to the wasm interface, must be called before the plugin can be used.
    unsafe fn register(name: &'static str, logo: Option<&'static [u8]>) {
        SAPIO_V1_WASM_PLUGIN_CLIENT_GET_CREATE_ARGUMENTS_PTR = Self::get_api_inner;
        SAPIO_V1_WASM_PLUGIN_CLIENT_CREATE_PTR = Self::create;
        SAPIO_PLUGIN_NAME = name;
        if let Some(logo) = logo {
            SAPIO_PLUGIN_LOGO = logo;
        }
    }
}

/// Helper function for encoding a JSON into WASM linear memory
fn encode_json<S: Serialize>(s: &S) -> *mut c_char {
    if let Ok(Ok(c)) = serde_json::to_string(s).map(CString::new) {
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
    [$plugin:ident$(, $logo:expr)?] => {
        REGISTER![[$plugin, $plugin]$(, $logo)*];
    };
    [[$to:ident,$plugin:ident]$(, $logo:expr)?] => {
        impl Plugin for $plugin {
            type ToType = $to;
        }
        #[no_mangle]
        unsafe fn sapio_v1_wasm_plugin_entry_point() {
            $plugin::register(stringify!($to), optional_logo!($($logo)*));
        }
    };
}

#[macro_export]
macro_rules! optional_logo {
    () => {
        None
    };
    ($logo:expr) => {
        Some(include_bytes!($logo))
    };
}
