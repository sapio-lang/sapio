use bitcoin::util::amount::{Amount, CoinAmount};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::convert::TryInto;

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug)]
pub struct AmountRange {
    min: CoinAmount,
    max: CoinAmount,
}
impl AmountRange {
    pub fn new() -> AmountRange {
        AmountRange {
            min: Amount::max_value().into(),
            max: Amount::min_value().into(),
        }
    }
    pub fn update_range(&mut self, amount: Amount) {
        // This is safe even though unwrap becuase min cannot be manually set to a nonsense value.
        self.min = std::cmp::min(self.min.try_into().unwrap(), amount).into();
        self.max = std::cmp::max(self.max.try_into().unwrap(), amount).into();
    }
    pub fn max(&self) -> CoinAmount {
        self.max
    }
}
