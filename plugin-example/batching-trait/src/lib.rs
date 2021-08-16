use sapio::contract::*;
use sapio::util::amountrange::*;
use sapio::*;
use sapio_trait::SapioJSONTrait;
use schemars::*;
use serde::*;
use std::collections::VecDeque;

/// A payment to a specific address
#[derive(JsonSchema, Serialize, Deserialize, Clone)]
pub struct Payment {
    /// The amount to send
    #[serde(with = "bitcoin::util::amount::serde::as_btc")]
    #[schemars(with = "f64")]
    pub amount: bitcoin::util::amount::Amount,
    /// # Address
    /// The Address to send to
    pub address: bitcoin::Address,
}
#[derive(Serialize, JsonSchema, Deserialize)]
pub struct BatchingTraitVersion0_1_1 {
    pub payments: Vec<Payment>,
    #[serde(with = "bitcoin::util::amount::serde::as_sat")]
    #[schemars(with = "u64")]
    pub feerate_per_byte: bitcoin::util::amount::Amount,
}

impl SapioJSONTrait for BatchingTraitVersion0_1_1 {
    fn get_example_for_api_checking() -> Self {
        BatchingTraitVersion0_1_1 {
            payments: vec![],
            feerate_per_byte: bitcoin::util::amount::Amount::from_sat(0),
        }
    }
}
