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
unsafe fn sapio_v1_wasm_plugin_client_create_nullptr(_c: *mut c_char) -> *mut c_char {
    panic!("No Function Registered");
}

/// a static mut that gets set when a Plugin::register method gets called
/// in order to enable binding when the type is registered
pub(crate) static mut sapio_v1_wasm_plugin_client_get_create_arguments_ptr: fn() -> *mut c_char =
    sapio_v1_wasm_plugin_client_get_create_arguments_nullptr;

/// a static mut that gets set when a Plugin::register method gets called
/// in order to enable binding when the type is registered
pub(crate) static mut sapio_v1_wasm_plugin_client_create_ptr: unsafe fn(
    *mut c_char,
) -> *mut c_char = sapio_v1_wasm_plugin_client_create_nullptr;

/// returns a pointer to the schema for the arguments required to create an instance
/// host must drop the returned pointer.
#[no_mangle]
extern "C" fn sapio_v1_wasm_plugin_client_get_create_arguments() -> *mut c_char {
    unsafe { sapio_v1_wasm_plugin_client_get_create_arguments_ptr() }
}

/// create an instance of the plugin's contract from the provided json args
/// host must drop the returned pointer.
#[no_mangle]
unsafe extern "C" fn sapio_v1_wasm_plugin_client_create(c: *mut c_char) -> *mut c_char {
    sapio_v1_wasm_plugin_client_create_ptr(c)
}

/// Drops a pointer that was created in the WASM
#[no_mangle]
unsafe extern "C" fn sapio_v1_wasm_plugin_client_drop_allocation(s: *mut c_char) {
    CString::from_raw(s);
}

/// Allows the host to allocate len bytes inside the WASM environment
/// Memory leaks if no call to sapio_v1_wasm_plugin_client_drop_allocation follows.
#[no_mangle]
extern "C" fn sapio_v1_wasm_plugin_client_allocate_bytes(len: u32) -> *mut c_char {
    CString::new(vec![1; len as usize]).unwrap().into_raw()
}

pub(crate) static mut sapio_plugin_name: &'static str = "Unnamed";
/// Gets a name for the plugin.
/// host must drop the returned pointer.
#[no_mangle]
unsafe extern "C" fn sapio_v1_wasm_plugin_client_get_name() -> *mut c_char {
    CString::new(sapio_plugin_name.as_bytes())
        .unwrap()
        .into_raw()
}
