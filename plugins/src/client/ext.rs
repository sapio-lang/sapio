// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! External symbols that must be provided by the WASM host

#[cfg_attr(target_arch = "wasm32", link(wasm_import_module = "env"))]
extern "C" {
    /// Write a diagnostic to the host's stderr. The host may make this a no-op.
    pub fn sapio_v1_wasm_plugin_debug_log_string(a: i32, len: i32);
    /// Create an instance of a contract by "trampolining" through the host to use another
    /// plugin identified by key.
    pub fn sapio_v1_wasm_plugin_create_contract(
        path: i32,
        path_len: i32,
        key: i32,
        json: i32,
        json_len: i32,
    ) -> i32;
    /// Get contract API by "trampolining" through the host to use another
    /// plugin identified by key.
    pub fn sapio_v1_wasm_plugin_get_api(key: i32) -> i32;
    /// Get contract logo by "trampolining" through the host to use another
    /// plugin identified by key.
    pub fn sapio_v1_wasm_plugin_get_logo(key: i32) -> i32;
    /// Get contract name by "trampolining" through the host to use another
    /// plugin identified by key.
    pub fn sapio_v1_wasm_plugin_get_name(key: i32) -> i32;
    /// lookup a plugin key from a human reable name.
    /// if ok == 1, result is valid.
    /// out is written and must be 32 bytes of writable memory.
    /// if name == 0 and name_len == 0, then return the current module
    pub fn sapio_v1_wasm_plugin_lookup_module_name(name: i32, name_len: i32, out: i32, ok: i32);
}

#[no_mangle]
fn now() -> f64 {
    0.0
}
