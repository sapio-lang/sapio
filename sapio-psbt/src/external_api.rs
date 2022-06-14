// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
use bitcoin::consensus::serialize;
use miniscript::psbt::PsbtExt;
use serde::{Deserialize, Serialize};

use bitcoin::secp256k1::Secp256k1;

use bitcoin::psbt::PartiallySignedTransaction;

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum PSBTApi {
    Finished {
        completed: bool,
        hex: String,
    },
    NotFinished {
        completed: bool,
        psbt: String,
        error: String,
        errors: Vec<String>,
    },
}

pub fn finalize_psbt_format_api(psbt: PartiallySignedTransaction) -> PSBTApi {
    let secp = Secp256k1::new();
    psbt.finalize(&secp)
        .map(|tx| {
            let hex = bitcoin::consensus::encode::serialize_hex(&tx.extract_tx());
            PSBTApi::Finished {
                completed: true,
                hex,
            }
        })
        .unwrap_or_else(|(psbt, errors)| {
            let errors: Vec<_> = errors.iter().map(|e| format!("{:?}", e)).collect();
            let encoded_psbt = base64::encode(serialize(&psbt));
            PSBTApi::NotFinished {
                completed: false,
                psbt: encoded_psbt,
                error: "Could not fully finalize psbt".into(),
                errors,
            }
        })
}
