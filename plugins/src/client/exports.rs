// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Functions that are made visible to the host to call inside the WASM module.
use super::*;

/// a stub to make the compiler happy
fn sapio_v1_wasm_plugin_client_get_create_arguments_nullptr() -> *mut c_char {
    panic!("No Function Registered");
}

/// a stub to make the compiler happy
unsafe fn sapio_v1_wasm_plugin_client_create_nullptr(
    _p: *mut c_char,
    _c: *mut c_char,
) -> *mut c_char {
    panic!("No Function Registered");
}

/// a static mut that gets set when a Plugin::register method gets called
/// in order to enable binding when the type is registered
pub(crate) static mut SAPIO_V1_WASM_PLUGIN_CLIENT_GET_CREATE_ARGUMENTS_PTR: fn() -> *mut c_char =
    sapio_v1_wasm_plugin_client_get_create_arguments_nullptr;

/// a static mut that gets set when a Plugin::register method gets called
/// in order to enable binding when the type is registered
pub(crate) static mut SAPIO_V1_WASM_PLUGIN_CLIENT_CREATE_PTR: unsafe fn(
    *mut c_char,
    *mut c_char,
) -> *mut c_char = sapio_v1_wasm_plugin_client_create_nullptr;

/// returns a pointer to the schema for the arguments required to create an instance
/// host must drop the returned pointer.
#[no_mangle]
extern "C" fn sapio_v1_wasm_plugin_client_get_create_arguments() -> *mut c_char {
    unsafe { SAPIO_V1_WASM_PLUGIN_CLIENT_GET_CREATE_ARGUMENTS_PTR() }
}

/// create an instance of the plugin's contract from the provided json args
/// host must drop the returned pointer.
#[no_mangle]
unsafe extern "C" fn sapio_v1_wasm_plugin_client_create(
    p: *mut c_char,
    c: *mut c_char,
) -> *mut c_char {
    SAPIO_V1_WASM_PLUGIN_CLIENT_CREATE_PTR(p, c)
}

/// Drops a pointer that was created in the WASM
#[no_mangle]
unsafe extern "C" fn sapio_v1_wasm_plugin_client_drop_allocation(s: *mut c_char) {
    // manually drop here for clarity / linter
    drop(CString::from_raw(s));
}

/// Allows the host to allocate len bytes inside the WASM environment
/// Memory leaks if no call to sapio_v1_wasm_plugin_client_drop_allocation follows.
#[no_mangle]
extern "C" fn sapio_v1_wasm_plugin_client_allocate_bytes(len: u32) -> *mut c_char {
    CString::new(vec![1; len as usize]).unwrap().into_raw()
}

pub(crate) static mut SAPIO_PLUGIN_NAME: &'static str = "Unnamed";

/// Gets a name for the plugin.
/// host must drop the returned pointer.
#[no_mangle]
unsafe extern "C" fn sapio_v1_wasm_plugin_client_get_name() -> *mut c_char {
    CString::new(SAPIO_PLUGIN_NAME.as_bytes())
        .unwrap()
        .into_raw()
}

pub(crate) static mut SAPIO_PLUGIN_LOGO: &'static [u8] = include_bytes!("logo.png");
/// Gets a name for the plugin.
/// host must drop the returned pointer.
#[no_mangle]
unsafe extern "C" fn sapio_v1_wasm_plugin_client_get_logo() -> *mut c_char {
    CString::new(Vec::<u8>::from(base64::encode(SAPIO_PLUGIN_LOGO)))
        .unwrap()
        .into_raw()
}
