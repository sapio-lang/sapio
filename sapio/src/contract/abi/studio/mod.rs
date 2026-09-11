// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Formats for Sapio Studio
use crate::contract::abi::continuation::ContinuationPoint;
use crate::contract::object::ObjectMetadata;
use crate::template::output::OutputMeta;
use crate::template::TemplateMetadata;
use bitcoin::psbt::Psbt;
use bitcoin::OutPoint;
use miniscript::*;
use sapio_base::serialization_helpers::SArc;
use sapio_base::{effects::EffectPath, miniscript};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Container for data from  `Object::bind_psbt`.
#[derive(Serialize, Deserialize)]
#[serde(rename = "linked_psbt")]
pub struct LinkedPSBT {
    /// a PSBT
    pub psbt: Psbt,
    /// tx level metadata
    pub metadata: TemplateMetadata,
    /// output specific metadata
    pub output_metadata: Vec<ObjectMetadata>,
    /// added metadata
    pub added_output_metadata: Vec<OutputMeta>,
}

/// Format for a Linked PSBT in Sapio Studio
#[derive(Serialize, Deserialize, JsonSchema, Debug)]
pub enum SapioStudioFormat {
    /// Used for PSBT Return Values
    #[serde(rename = "linked_psbt")]
    LinkedPSBT {
        /// Base 64 Encoded PSBT
        psbt: String,
        /// Hex encoded TXN
        hex: String,
        /// tx level metadata
        metadata: TemplateMetadata,
        /// output specific metadata
        output_metadata: Vec<ObjectMetadata>,
        /// added metadata
        added_output_metadata: Vec<OutputMeta>,
    },
}

impl From<LinkedPSBT> for SapioStudioFormat {
    fn from(l: LinkedPSBT) -> SapioStudioFormat {
        let psbt = {
            let bytes = l.psbt.serialize();
            base64::encode(bytes)
        };
        // Studio previews may be unsigned or incomplete; this is not a
        // completed-spend export or a fee-policy acceptance check.
        let hex =
            bitcoin::consensus::encode::serialize_hex(&l.psbt.extract_tx_unchecked_fee_rate());
        SapioStudioFormat::LinkedPSBT {
            psbt,
            hex,
            metadata: l.metadata,
            output_metadata: l.output_metadata,
            added_output_metadata: l.added_output_metadata,
        }
    }
}

/// Bound contract occurrences with their PSBTs, outputs and continuation APIs.
///
/// Map keys identify occurrences in the bound graph. A compiled contract can
/// appear at several outputs; its original path is retained in `source_path`.
#[derive(Serialize, Deserialize, Debug, JsonSchema)]
pub struct Program {
    /// Entries keyed by their bound occurrence paths. Child paths derive from
    /// the parent, transition kind, template hash and output index.
    pub program: BTreeMap<SArc<EffectPath>, SapioStudioObject>,
}

/// A `SapioStudioObject` is a json-friendly format for a `Object` for use in Sapio Studio
#[derive(Serialize, Deserialize, Debug, JsonSchema)]
pub struct SapioStudioObject {
    /// Original compilation path, independent of this occurrence's binding
    /// path. Synthetic funding entries have no compilation source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<SArc<EffectPath>>,
    /// The object's metadata
    pub metadata: ObjectMetadata,
    /// The main covenant OutPoint
    pub out: OutPoint,
    /// List of SapioStudioFormat PSBTs
    pub txs: Vec<SapioStudioFormat>,
    /// List of continue APIs from this point.
    pub continue_apis: BTreeMap<SArc<EffectPath>, ContinuationPoint>,
}
