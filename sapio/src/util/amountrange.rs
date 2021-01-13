use bitcoin::util::amount;
use bitcoin::util::amount::{Amount, CoinAmount};
use bitcoin::SignedAmount;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::convert::TryInto;

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Ord, PartialOrd, PartialEq, Eq)]
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

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug)]
pub struct AmountRange {
    min: Option<AmountF64>,
    max: Option<AmountF64>,
}
impl AmountRange {
    pub fn new() -> AmountRange {
        AmountRange {
            min: None,
            max: None,
        }
    }
    pub fn update_range(&mut self, amount: Amount) {
        self.min = std::cmp::min(self.min, Some(amount.into()));
        self.max = std::cmp::max(self.max, Some(amount.into()));
    }
    pub fn max(&self) -> Amount {
        self.max.unwrap_or(Amount::min_value().into()).0
    }
}
