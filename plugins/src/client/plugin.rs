// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! binding for making a type into a plugin
use super::*;
use sapio::contract::CompilationError;
use sapio_base::effects::EffectPath;
use sapio_base::effects::PathFragment;
use sapio_base::serialization_helpers::SArc;

use std::convert::TryFrom;

/// Represents any type which can be treated as a module
pub trait Callable {
    /// The result type to be produced
    type Output;
    /// Call the function
    fn call(&self, ctx: Context) -> Result<Self::Output, CompilationError>;
}
impl<T> Callable for T
where
    T: Compilable,
{
    type Output = Compiled;
    fn call(&self, ctx: Context) -> Result<Compiled, CompilationError> {
        self.compile(ctx)
    }
}

/// The `Plugin` trait is used to provide bindings for a WASM Plugin.
/// It's not intended to be used internally, just as bindings.
pub trait Plugin
where
    // Self must be Callable and produce an Output. We must also be able
    // to get one from Self::InputWrapper, potentially falliably
    Self: Callable + TryFrom<Self::InputWrapper>,
    // InputWrapper must be deserializable and describable
    Self::InputWrapper: JsonSchema + Sized + for<'a> Deserialize<'a>,
    // read as: The return type of CallableType::try_from(self) is
    // Result<CallableType, X>, where X must be able to x.into() a
    // CompilationError.
    CompilationError: From<<Self as TryFrom<Self::InputWrapper>>::Error>,
    // We must be able to serialize/describe the outputs
    Self::Output: Serialize + JsonSchema,
{
    /// A type which wraps Self, but can be converted into Self.
    type InputWrapper;
    /// gets the jsonschema for the plugin type, which is the API for calling create.
    fn get_api_inner() -> *mut c_char {
        encode_json(&API::<CreateArgs<Self::InputWrapper>, Self::Output>::new())
    }

    /// creates an instance of the plugin from a json pointer and outputs a result pointer
    unsafe fn create(p: *mut c_char, c: *mut c_char) -> *mut c_char {
        let res = Self::create_result(p, c).map_err(|e| e.to_string());
        encode_json(&res)
    }

    /// creates an instance of the plugin from a json pointer and outputs a typed result
    unsafe fn create_result(
        p: *mut c_char,
        c: *mut c_char,
    ) -> Result<Self::Output, CompilationError> {
        let s = CString::from_raw(c);
        let path = CString::from_raw(p);
        let CreateArgs::<Self::InputWrapper> {
            arguments,
            context:
                ContextualArguments {
                    network,
                    amount,
                    effects,
                    ordinals_info
                },
        } = serde_json::from_slice(s.to_bytes()).map_err(CompilationError::DeserializationError)?;
        // TODO: In theory, these trampoline bounds are robust/serialization safe...
        // But the API needs stiching to the parent in a sane way...
        let caller = lookup_this_module_name()
            .map(|s| bitcoin::hashes::hex::ToHex::to_hex(&s[..]))
            .ok_or_else(|| {
                CompilationError::InternalModuleError(
                    "Host Error: Should always be able to identify module's own ID".into(),
                )
            })?;
        let cstring_to_string = path
            .to_str()
            .map_err(|e| CompilationError::InternalModuleError(format!("Path Invalid: {}", e)))?;

        let parsed_rpath = serde_json::from_str(cstring_to_string)
            .map_err(CompilationError::DeserializationError)?;
        let path: EffectPath = EffectPath::push_owned(
            Some(EffectPath::push(
                Some(Arc::new(parsed_rpath)),
                PathFragment::Root,
            )),
            PathFragment::Named(SArc(Arc::new(caller))),
        );

        let ctx = Context::new(
            network,
            amount,
            Arc::new(client::WasmHostEmulator),
            path,
            // TODO: load database?
            Arc::new(effects),
            ordinals_info
        );
        let converted = Self::try_from(arguments)?;
        converted.call(ctx)
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
        std::ptr::null_mut::<c_char>()
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
    [[$to:ident,$wrapper:ident]$(, $logo:expr)?] => {
        const _ : () = {
            use sapio_wasm_plugin::client::Plugin;
            use sapio_wasm_plugin::client::plugin::Callable;
            use schemars::JsonSchema;
            use serde::*;
            use core::convert::TryFrom;
            use sapio::contract::CompilationError;
            use sapio::Context;
            #[derive(Deserialize, JsonSchema)]
            #[serde(transparent)]
            struct SapioInternalWrapperAroundInput($wrapper);
            impl TryFrom<SapioInternalWrapperAroundInput> for SapioInternalWrapperAroundCallable {
                type Error = <$to as TryFrom<$wrapper>>::Error;
                fn try_from(v: SapioInternalWrapperAroundInput) -> Result<SapioInternalWrapperAroundCallable, Self::Error> {
                    $to::try_from(v.0).map(SapioInternalWrapperAroundCallable)
                }
            }

            struct SapioInternalWrapperAroundCallable($to);
            impl Callable for SapioInternalWrapperAroundCallable {
                type Output = <$to as Callable>::Output;
                fn call(&self, ctx: Context) -> Result<Self::Output, CompilationError> {
                    self.0.call(ctx)
                }
            }

            impl Plugin for SapioInternalWrapperAroundCallable {
                type InputWrapper = SapioInternalWrapperAroundInput;
            }
            #[no_mangle]
            unsafe fn sapio_v1_wasm_plugin_entry_point() {
                SapioInternalWrapperAroundCallable::register(stringify!($to), optional_logo!($($logo)*));
            }
        };
    };
}

/// If a logo file is specified, use it.
#[macro_export]
macro_rules! optional_logo {
    () => {
        None
    };
    ($logo:expr) => {
        Some(include_bytes!($logo))
    };
}
