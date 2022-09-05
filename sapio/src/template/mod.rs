// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! utilities for building Bitcoin transaction templates up programmatically
use crate::contract::error::CompilationError;
use bitcoin::hashes::sha256;
use bitcoin::util::amount::Amount;
use sapio_base::simp::SIMPAttachableAt;
use sapio_base::simp::SIMPError;
use sapio_base::simp::TemplateInputLT;
use sapio_base::simp::TemplateLT;
use sapio_base::Clause;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub mod input;
pub mod output;
pub use output::{Output, OutputMeta};
pub mod builder;
pub use builder::Builder;

use self::input::InputMetadata;
/// Metadata Struct which has some standard defined fields
/// and can be extended via a hashmap
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub struct TemplateMetadata {
    /// A Label for this transaction
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub label: Option<String>,
    /// catch all map for future metadata....
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
    /// SIMP: Sapio Interactive Metadata Protocol
    pub simp: BTreeMap<i64, serde_json::Value>,
    /// A Color to render this node.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub color: Option<String>,
}

impl TemplateMetadata {
    /// helps determine if a TemplateMetadata has anything worth serializing or not
    pub fn skip_serializing(&self) -> bool {
        *self == TemplateMetadata::new()
    }
    /// create a new `TemplateMetadata`
    pub fn new() -> Self {
        TemplateMetadata {
            simp: BTreeMap::new(),
            color: None,
            label: None,
            extra: BTreeMap::new(),
        }
    }
    /// set an extra metadata value
    pub fn set_extra<I, J>(mut self, i: I, j: J) -> Result<Self, CompilationError>
    where
        I: Into<String>,
        J: Into<serde_json::Value>,
    {
        let s: String = i.into();
        match s.as_str() {
            "color" | "label" => Err(CompilationError::TerminateWith(
                "Don't Set label or color through the extra API".into(),
            )),
            _ => {
                if self.extra.insert(s.clone(), j.into()).is_some() {
                    return Err(CompilationError::OverwriteMetadata(s));
                }
                Ok(self)
            }
        }
    }
    /// set a color
    pub fn set_color<I>(mut self, i: I) -> Result<Self, CompilationError>
    where
        I: Into<String>,
    {
        if self.color.is_some() {
            return Err(CompilationError::OverwriteMetadata("color".into()));
        }
        self.color = Some(i.into());
        Ok(self)
    }
    /// set a label
    pub fn set_label<I>(mut self, i: I) -> Result<Self, CompilationError>
    where
        I: Into<String>,
    {
        if self.label.is_some() {
            return Err(CompilationError::OverwriteMetadata("label".into()));
        }
        self.label = Some(i.into());
        Ok(self)
    }

    /// attempts to add a SIMP to the output meta.
    ///
    /// Returns [`SIMPError::AlreadyDefined`] if one was previously set.
    pub fn add_simp<S: SIMPAttachableAt<TemplateLT>>(mut self, s: S) -> Result<Self, SIMPError> {
        let old = self.simp.insert(s.get_protocol_number(), s.to_json()?);
        if let Some(old) = old {
            Err(SIMPError::AlreadyDefined(old))
        } else {
            Ok(self)
        }
    }
}

/// Template holds the data needed to construct a Transaction for CTV Purposes, along with relevant
/// metadata
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Template {
    /// additional restrictions placed on this template
    #[serde(rename = "additional_preconditions")]
    pub guards: Vec<Clause>,
    /// the precomputed template hash for this Template
    #[serde(rename = "precomputed_template_hash")]
    pub ctv: sha256::Hash,
    /// the index used for the template hash. (TODO: currently always 0, although
    /// future version may support other indexes)
    #[serde(rename = "precomputed_template_hash_idx")]
    pub ctv_index: u32,
    /// the amount being sent to this Template (TODO: currently computed via tx.total_amount())
    #[serde(
        rename = "max_amount_sats",
        with = "bitcoin::util::amount::serde::as_sat"
    )]
    #[schemars(with = "i64")]
    pub max: Amount,
    /// the amount being sent to this Template (TODO: currently computed via tx.total_amount())
    #[serde(
        rename = "min_feerate_sats_vbyte",
        with = "bitcoin::util::amount::serde::as_sat::opt"
    )]
    #[schemars(with = "Option<i64>")]
    pub min_feerate_sats_vbyte: Option<Amount>,
    /// any metadata fields attached to this template
    #[serde(
        skip_serializing_if = "TemplateMetadata::skip_serializing",
        default = "TemplateMetadata::new"
    )]
    pub metadata_map_s2s: TemplateMetadata,
    /// The actual transaction this template will create
    #[serde(rename = "transaction_literal")]
    pub tx: bitcoin::Transaction,
    /// sapio specific information about all the outputs in the `tx`.
    #[serde(rename = "outputs_info")]
    pub outputs: Vec<Output>,
    /// sapio specific information about all the inputs in the `tx`.
    #[serde(rename = "inputs_info")]
    pub inputs: Vec<InputMetadata>,
}

impl Template {
    /// Get the cached template hash of this Template
    pub fn hash(&self) -> sha256::Hash {
        self.ctv
    }

    /// recompute the total amount spent in this template. This is the total
    /// amount required to be sent to this template for this transaction to
    /// succeed.
    pub fn total_amount(&self) -> Amount {
        self.outputs
            .iter()
            .map(|o| o.amount)
            .fold(Amount::from_sat(0), |b, a| b + a)
    }
}
