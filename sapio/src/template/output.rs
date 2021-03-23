// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Template Output container
use super::*;
/// Metadata for outputs, arbitrary KV set.
pub type OutputMeta = HashMap<String, String>;

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
        skip_serializing_if = "HashMap::is_empty",
        default
    )]
    pub metadata: OutputMeta,
}
