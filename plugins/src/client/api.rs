// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

///! Wraps the external API with friendly methods
use super::*;
use bitcoin::Amount;
/// Print a &str to the parent's console.
pub fn log(s: &str) {
    unsafe {
        sapio_v1_wasm_plugin_debug_log_string(s.as_ptr() as i32, s.len() as i32);
    }
}

/// Given a 32 byte plugin identifier, create a new contract instance.
pub fn create_contract_by_key(key: &[u8; 32], args: Value, amt: Amount) -> Option<Compiled> {
    unsafe {
        let s = args.to_string();
        let l = s.len();
        let p = sapio_v1_wasm_plugin_create_contract(
            key.as_ptr() as i32,
            s.as_ptr() as i32,
            l as i32,
            amt.as_sat() as u32,
        );
        if p != 0 {
            let cs = CString::from_raw(p as *mut c_char);
            serde_json::from_slice(cs.as_bytes()).ok()
        } else {
            None
        }
    }
}

/// lookup a plugin module's key given a human readable name
pub fn lookup_module_name(key: &str) -> Option<[u8; 32]> {
    unsafe {
        let mut res = [0u8; 32];
        let mut ok = 0u8;
        sapio_v1_wasm_plugin_lookup_module_name(
            key.as_ptr() as i32,
            key.len() as i32,
            &mut res as *mut [u8; 32] as i32,
            &mut ok as *mut u8 as i32,
        );
        if ok == 0 {
            None
        } else {
            Some(res)
        }
    }
}

/// Given a human readable name, create a new contract instance
pub fn create_contract(key: &str, args: Value, amt: Amount) -> Option<Compiled> {
    let key = lookup_module_name(key)?;
    create_contract_by_key(&key, args, amt)
}

/// A empty type tag to bind the dynamically linked host emulator functionality
pub struct WasmHostEmulator;
impl CTVEmulator for WasmHostEmulator {
    fn get_signer_for(
        &self,
        h: bitcoin::hashes::sha256::Hash,
    ) -> std::result::Result<
        miniscript::policy::concrete::Policy<bitcoin::PublicKey>,
        sapio_ctv_emulator_trait::EmulatorError,
    > {
        let mut inner = h.into_inner();
        let signer = unsafe {
            sapio_v1_wasm_plugin_ctv_emulator_signer_for(&mut inner[0] as *mut u8 as i32)
        };
        let signer = unsafe { CString::from_raw(signer as *mut c_char) };
        Ok(serde_json::from_slice(signer.to_bytes()).unwrap())
    }
    fn sign(
        &self,
        psbt: bitcoin::util::psbt::PartiallySignedTransaction,
    ) -> std::result::Result<
        bitcoin::util::psbt::PartiallySignedTransaction,
        sapio_ctv_emulator_trait::EmulatorError,
    > {
        let s = serde_json::to_string_pretty(&psbt).unwrap();
        let len = s.len();
        let ret = unsafe {
            CString::from_raw(
                sapio_v1_wasm_plugin_ctv_emulator_sign(s.as_ptr() as i32, len as u32)
                    as *mut c_char,
            )
        };
        let j = serde_json::from_slice(ret.as_bytes()).unwrap();
        Ok(j)
    }
}
