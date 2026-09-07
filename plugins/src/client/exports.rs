// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Functions that are made visible to the host to call inside the WASM module.
use super::*;

use std::sync::OnceLock;

/// Publish the complete registration in one step before exposing any callback.
pub(crate) struct PluginBindings {
    pub get_arguments: fn() -> *mut c_char,
    pub create: unsafe fn(*mut c_char, *mut c_char) -> *mut c_char,
    pub name: &'static str,
    pub logo: &'static [u8],
}

pub(crate) static PLUGIN: OnceLock<PluginBindings> = OnceLock::new();

fn registered() -> &'static PluginBindings {
    PLUGIN
        .get()
        .expect("Plugin entry point must register before use")
}

/// returns a pointer to the schema for the arguments required to create an instance
/// host must drop the returned pointer.
#[no_mangle]
extern "C" fn sapio_v1_wasm_plugin_client_get_create_arguments() -> *mut c_char {
    (registered().get_arguments)()
}

/// create an instance of the plugin's contract from the provided json args
/// host must drop the returned pointer.
#[no_mangle]
unsafe extern "C" fn sapio_v1_wasm_plugin_client_create(
    p: *mut c_char,
    c: *mut c_char,
) -> *mut c_char {
    (registered().create)(p, c)
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

/// Gets a name for the plugin.
/// host must drop the returned pointer.
#[no_mangle]
extern "C" fn sapio_v1_wasm_plugin_client_get_name() -> *mut c_char {
    CString::new(registered().name.as_bytes())
        .unwrap()
        .into_raw()
}

/// Gets a name for the plugin.
/// host must drop the returned pointer.
#[no_mangle]
extern "C" fn sapio_v1_wasm_plugin_client_get_logo() -> *mut c_char {
    CString::new(Vec::<u8>::from(base64::encode(registered().logo)))
        .unwrap()
        .into_raw()
}
