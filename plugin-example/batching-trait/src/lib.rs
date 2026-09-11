use sapio_wasm_plugin::client::*;
use schemars::*;
use serde::*;
/// A payment to a specific address
#[derive(JsonSchema, Serialize, Deserialize, Clone)]
pub struct Payment {
    /// The amount to send
    #[serde(with = "bitcoin::amount::serde::as_btc")]
    #[schemars(with = "f64")]
    pub amount: bitcoin::Amount,
    /// # Address
    /// The Address to send to
    pub address: bitcoin::Address<bitcoin::address::NetworkUnchecked>,
}
#[derive(Serialize, JsonSchema, Deserialize, Clone)]
pub struct BatchingTraitVersion0_1_1 {
    pub payments: Vec<Payment>,
    #[serde(with = "bitcoin::amount::serde::as_sat")]
    #[schemars(with = "u64")]
    pub feerate_per_byte: bitcoin::Amount,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub enum Versions {
    BatchingTraitVersion0_1_1(BatchingTraitVersion0_1_1),
}

pub type BatchingModule = ContractModule<Versions>;
