// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

///! Host based Emulator
use crate::client::ext::*;
use bitcoin::hashes::Hash;
use sapio_ctv_emulator_trait::CTVEmulator;
use std::ffi::CString;
use std::os::raw::c_char;

/// A empty type tag to bind the dynamically linked host emulator functionality
pub struct WasmHostEmulator;
impl CTVEmulator for WasmHostEmulator {
    fn get_signer_for(
        &self,
        h: bitcoin::hashes::sha256::Hash,
    ) -> std::result::Result<
        miniscript::policy::concrete::Policy<bitcoin::XOnlyPublicKey>,
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
