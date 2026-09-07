// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
use crate::{validate_psbt, PSBTValidationError};
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

/// Finalize a structurally valid PSBT, retaining incomplete signing results.
/// Malformed transaction and metadata fields return a validation error.
pub fn finalize_psbt_format_api(
    psbt: PartiallySignedTransaction,
) -> Result<PSBTApi, PSBTValidationError> {
    validate_psbt(&psbt)?;
    let secp = Secp256k1::new();
    Ok(psbt
        .finalize(&secp)
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
        }))
}
