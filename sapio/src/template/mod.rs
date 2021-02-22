use bitcoin::hashes::sha256;
use bitcoin::util::amount::Amount;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub mod output;
pub use output::{Output, OutputMeta};

pub mod builder;
pub use builder::Builder;

/// Metadata Struct which has some standard defined fields
/// and can be extended via a hashmap
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct TemplateMetadata {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    label: Option<String>,
    #[serde(flatten)]
    extra: HashMap<String, String>,
}

impl TemplateMetadata {
    pub fn skip_serializing(&self) -> bool {
        self.label.is_none() && self.extra.is_empty()
    }
    pub fn new() -> Self {
        TemplateMetadata {
            label: None,
            extra: HashMap::new(),
        }
    }
}

/// Template holds the data needed to construct a Transaction for CTV Purposes, along with relevant
/// metadata
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Template {
    #[serde(rename = "precomputed_template_hash")]
    pub ctv: sha256::Hash,
    #[serde(rename = "precomputed_template_hash_idx")]
    pub ctv_index: u32,
    #[serde(
        rename = "max_amount_sats",
        with = "bitcoin::util::amount::serde::as_sat"
    )]
    #[schemars(with = "i64")]
    pub max: Amount,
    #[serde(
        skip_serializing_if = "TemplateMetadata::skip_serializing",
        default = "TemplateMetadata::new"
    )]
    pub metadata_map_s2s: TemplateMetadata,
    #[serde(rename = "transaction_literal")]
    pub tx: bitcoin::Transaction,
    #[serde(rename = "outputs_info")]
    pub outputs: Vec<Output>,
}

impl Template {
    pub fn hash(&self) -> sha256::Hash {
        self.ctv
    }

    pub fn total_amount(&self) -> Amount {
        self.outputs
            .iter()
            .map(|o| o.amount)
            .fold(Amount::from_sat(0), |b, a| b + a)
    }
}
