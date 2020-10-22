use bitcoin::util::amount::Amount;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy)]
pub struct AmountRange {
    pub min: u64,
    pub max: u64,
}
impl AmountRange {
    pub fn new() -> AmountRange {
        AmountRange {
            min: Amount::max_value().as_sat(),
            max: Amount::min_value().as_sat(),
        }
    }
    pub fn update_range(&mut self, amount: Amount) {
        self.min = std::cmp::min(self.min, amount.as_sat());
        self.max = std::cmp::max(self.max, amount.as_sat());
    }
}
