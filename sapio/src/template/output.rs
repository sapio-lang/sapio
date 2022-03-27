// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Template Output container
use super::*;
use serde::{Deserialize, Serialize};
/// Metadata for outputs, arbitrary KV set.
#[derive(Serialize, Deserialize, Clone, JsonSchema, Debug)]
pub struct OutputMeta {
    /// Additional non-standard fields for future upgrades
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}
impl OutputMeta {
    fn is_empty(&self) -> bool {
        self.extra.is_empty()
    }
}
impl Default for OutputMeta {
    fn default() -> Self {
        OutputMeta {
            extra: Default::default(),
        }
    }
}

impl<const N: usize> From<[(&str, serde_json::Value); N]> for OutputMeta {
    fn from(v: [(&str, serde_json::Value); N]) -> OutputMeta {
        OutputMeta {
            extra: IntoIterator::into_iter(v)
                .map(|(a, b)| (a.into(), b))
                .collect(),
        }
    }
}

/// An Output is not a literal Bitcoin Output, but contains data needed to construct one, and
/// metadata for linking & ABI building
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Output {
    /// the amount of sats being sent to this contract
    #[serde(with = "bitcoin::util::amount::serde::as_sat")]
    #[schemars(with = "i64")]
    #[serde(rename = "sending_amount_sats")]
    pub amount: Amount,
    /// the compiled contract this output creates
    #[serde(rename = "receiving_contract")]
    pub contract: crate::contract::Compiled,
    /// any metadata relevant to this contract
    #[serde(
        rename = "metadata_map_s2s",
        skip_serializing_if = "OutputMeta::is_empty",
        default
    )]
    pub metadata: OutputMeta,
}
