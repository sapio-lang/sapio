// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

///! Various utils for working with modules
use super::*;

use sapio::contract::CompilationError;
use sapio_base::effects::EffectPath;

/// Print a &str to the parent's console.
pub fn log(s: &str) {
    unsafe {
        sapio_v1_wasm_plugin_debug_log_string(s.as_ptr() as i32, s.len() as i32);
    }
}

/// Given a 32 byte plugin identifier, create a new contract instance.
pub fn call_path<S: Serialize, T>(
    path: &EffectPath,
    key: &[u8; 32],
    args: CreateArgs<S>,
) -> Result<T, CompilationError>
where
    T: for<'a> Deserialize<'a> + JsonSchema,
{
    let path = serde_json::to_string(path).map_err(CompilationError::SerializationError)?;
    let s = serde_json::to_value(args)
        .map_err(CompilationError::SerializationError)?
        .to_string();
    let l = s.len();
    let p = unsafe {
        sapio_v1_wasm_plugin_create_contract(
            path.as_ptr() as i32,
            path.len() as i32,
            key.as_ptr() as i32,
            s.as_ptr() as i32,
            l as i32,
        )
    };
    if p != 0 {
        let cs = unsafe { CString::from_raw(p as *mut c_char) };
        let res: Result<T, String> = serde_json::from_slice(cs.as_bytes())
            .map_err(CompilationError::DeserializationError)?;
        res.map_err(CompilationError::ModuleCompilationErrorUnsendable)
    } else {
        Err(CompilationError::InternalModuleError("Unknown".into()))
    }
}

pub fn get_logo(key: &[u8; 32]) -> Result<String, CompilationError> {
    let p = key.as_ptr() as i32;
    let logo: Result<String, String> = {
        let buf = unsafe { sapio_v1_wasm_plugin_get_logo(p) };
        if buf == 0 {
            return Err(CompilationError::InternalModuleError(
                "Logo Not Available".into(),
            ));
        }
        let cs = unsafe { CString::from_raw(buf as *mut c_char) };
        serde_json::from_slice(cs.as_bytes()).map_err(CompilationError::DeserializationError)?
    };
    logo.map_err(CompilationError::InternalModuleError)
}

pub fn get_name(key: &[u8; 32]) -> Result<String, CompilationError> {
    let p = key.as_ptr() as i32;
    let name: Result<String, String> = {
        let buf = unsafe { sapio_v1_wasm_plugin_get_name(p) };
        if buf == 0 {
            return Err(CompilationError::InternalModuleError(
                "Name Not Available".into(),
            ));
        }
        let cs = unsafe { CString::from_raw(buf as *mut c_char) };
        serde_json::from_slice(cs.as_bytes()).map_err(CompilationError::DeserializationError)?
    };
    name.map_err(CompilationError::InternalModuleError)
}

pub fn get_api<T, R>(key: &[u8; 32]) -> Result<API<T, R>, CompilationError> {
    let p = key.as_ptr() as i32;
    let api: Result<API<T, R>, String> = {
        let api_buf = unsafe { sapio_v1_wasm_plugin_get_api(p) };
        if api_buf == 0 {
            return Err(CompilationError::InternalModuleError(
                "API Not Available".into(),
            ));
        }
        let cs = unsafe { CString::from_raw(api_buf as *mut c_char) };
        serde_json::from_slice(cs.as_bytes()).map_err(CompilationError::DeserializationError)?
    };
    api.map_err(CompilationError::InternalModuleError)
}

pub fn call<S: Serialize, T>(
    ctx: Context,
    key: &[u8; 32],
    args: CreateArgs<S>,
) -> Result<T, CompilationError>
where
    T: for<'a> Deserialize<'a> + JsonSchema,
{
    call_path(ctx.path(), key, args)
}

/// lookup a plugin module's key given a human readable name
pub fn lookup_module_name(key: &str) -> Option<[u8; 32]> {
    let mut res = [0u8; 32];
    let mut ok = 0u8;
    unsafe {
        sapio_v1_wasm_plugin_lookup_module_name(
            key.as_ptr() as i32,
            key.len() as i32,
            &mut res as *mut [u8; 32] as i32,
            &mut ok as *mut u8 as i32,
        )
    };
    if ok == 0 {
        None
    } else {
        Some(res)
    }
}

/// Get the current executing module's hash
pub fn lookup_this_module_name() -> Option<[u8; 32]> {
    let mut res = [0u8; 32];
    let mut ok = 0u8;
    unsafe {
        sapio_v1_wasm_plugin_lookup_module_name(
            0i32,
            0i32,
            &mut res as *mut [u8; 32] as i32,
            &mut ok as *mut u8 as i32,
        )
    };
    if ok == 0 {
        None
    } else {
        Some(res)
    }
}

/// Given a human readable name, create a new contract instance
pub fn create_contract<S: Serialize>(
    context: Context,
    key: &str,
    args: CreateArgs<S>,
) -> Result<Compiled, CompilationError> {
    let key = lookup_module_name(key).ok_or(CompilationError::UnknownModule)?;
    call(context, &key, args)
}
