// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Functionality for working with ranges of amounts
use bitcoin::util::amount::Amount;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A wrapper around `bitcoin::Amount` to force it to serialize with f64.
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Ord, PartialOrd, PartialEq, Eq,
)]
#[serde(transparent)]
struct AmountF64(
    #[schemars(with = "f64")]
    #[serde(with = "bitcoin::util::amount::serde::as_btc")]
    Amount,
);

impl From<Amount> for AmountF64 {
    fn from(a: Amount) -> AmountF64 {
        AmountF64(a)
    }
}
impl From<AmountF64> for Amount {
    fn from(a: AmountF64) -> Amount {
        a.0
    }
}

/// `AmountRange` makes it simple to track and update the range of allowed values
/// for a contract to receive.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug)]
pub struct AmountRange {
    #[serde(rename = "min_btc", skip_serializing_if = "Option::is_none", default)]
    min: Option<AmountF64>,
    #[serde(rename = "max_btc", skip_serializing_if = "Option::is_none", default)]
    max: Option<AmountF64>,
}
impl AmountRange {
    /// create a new AmountRange with no set values
    pub fn new() -> AmountRange {
        AmountRange {
            min: None,
            max: None,
        }
    }
    /// Update the min and the max value.
    pub fn update_range(&mut self, amount: Amount) {
        self.min = std::cmp::min(self.min, Some(amount.into()));
        self.max = std::cmp::max(self.max, Some(amount.into()));
    }
    /// Retreive the max value, if set, or return `Amount::min_value`.
    pub fn max(&self) -> Amount {
        self.max.unwrap_or(Amount::min_value().into()).0
    }
}
