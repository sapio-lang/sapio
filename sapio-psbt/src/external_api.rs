// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
use crate::{validate_psbt, PSBTValidationError};
use bitcoin::consensus::serialize;
use bitcoin::util::taproot::{LeafVersion, TapLeafHash};
use miniscript::psbt::PsbtExt;
use miniscript::{Miniscript, Tap};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

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
/// A custom tapscript leaf does not prevent using another satisfiable leaf.
/// If finalization fails, unsupported leaves are identified in the returned
/// diagnostics and their PSBT data remains available to an external satisfier.
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
            let mut errors: Vec<_> = errors.iter().map(|e| format!("{:?}", e)).collect();
            for (index, input) in psbt.inputs.iter().enumerate() {
                let unsupported: BTreeSet<_> = input
                    .tap_scripts
                    .values()
                    .filter(|(script, version)| {
                        *version == LeafVersion::TapScript
                            && Miniscript::<bitcoin::XOnlyPublicKey, Tap>::parse_insane(script)
                                .is_err()
                    })
                    .map(|(script, version)| TapLeafHash::from_script(script, *version))
                    .collect();
                for leaf in unsupported {
                    errors.push(format!(
                        "Input {index}: unsupported custom tapscript leaf {leaf}; spending this leaf requires an external satisfier"
                    ));
                }
            }
            let encoded_psbt = base64::encode(serialize(&psbt));
            PSBTApi::NotFinished {
                completed: false,
                psbt: encoded_psbt,
                error: "Could not fully finalize psbt".into(),
                errors,
            }
        }))
}
