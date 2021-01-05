use bitcoin::hashes::sha256;
use bitcoin::util::amount::Amount;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub mod output;
pub use output::{Output, OutputMeta};

pub mod builder;
pub use builder::Builder;

/// Template holds the data needed to construct a Transaction for CTV Purposes, along with relevant
/// metadata
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Template {
    pub outputs: Vec<Output>,
    pub tx: bitcoin::Transaction,
    pub ctv: sha256::Hash,
    #[serde(with = "bitcoin::util::amount::serde::as_sat")]
    #[schemars(with = "i64")]
    pub max: Amount,
    pub label: String,
}

impl Template {
    pub fn hash(&self) -> sha256::Hash {
        self.ctv
    }

    pub fn total_amount(&self) -> Amount {
        Amount::from_sat(0)
    }
}
