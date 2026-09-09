// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Explicit BTC and satoshi serialization wrappers for amounts
use bitcoin::util::amount::Amount;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A wrapper around `bitcoin::Amount` to force it to serialize with f64.
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Ord, PartialOrd, PartialEq, Eq,
)]
#[serde(transparent)]
pub struct AmountF64(
    /// # Amount (BTC)
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
/// A wrapper around `bitcoin::Amount` to force it to serialize with u64.
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Ord, PartialOrd, PartialEq, Eq,
)]
#[serde(transparent)]
pub struct AmountU64(
    /// # Amount (Sats)
    #[schemars(with = "u64")]
    #[serde(with = "bitcoin::util::amount::serde::as_sat")]
    Amount,
);

impl From<Amount> for AmountU64 {
    fn from(a: Amount) -> AmountU64 {
        AmountU64(a)
    }
}
impl From<u64> for AmountU64 {
    fn from(a: u64) -> Self {
        AmountU64(Amount::from_sat(a))
    }
}
impl From<AmountU64> for Amount {
    fn from(a: AmountU64) -> Amount {
        a.0
    }
}
impl From<AmountU64> for u64 {
    fn from(a: AmountU64) -> u64 {
        a.0.as_sat()
    }
}
