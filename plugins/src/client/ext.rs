extern "C" {
    pub fn sapio_v1_wasm_plugin_ctv_emulator_sign(psbt: i32, len: u32) -> i32;
    pub fn sapio_v1_wasm_plugin_ctv_emulator_signer_for(hash: i32) -> i32;
    pub fn sapio_v1_wasm_plugin_debug_log_string(a: i32, len: i32);
    pub fn sapio_v1_wasm_plugin_create_contract(
        key: i32,
        json: i32,
        json_len: i32,
        amt: u32,
    ) -> i32;
    pub fn sapio_v1_wasm_plugin_lookup_module_name(key: i32, len: i32, out: i32, ok: i32);
}
