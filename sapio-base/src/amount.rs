//! Explicit denominations for user-supplied contract amounts.

use bitcoin::{amount::ParseAmountError, Amount};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// An amount whose JSON representation identifies its denomination.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize, JsonSchema)]
pub enum CoinAmount {
    /// An exact integer number of satoshis.
    Sats(u64),
    /// A Bitcoin amount, checked for range and sub-satoshi precision on conversion.
    Btc(f64),
}

impl TryFrom<CoinAmount> for Amount {
    type Error = ParseAmountError;

    fn try_from(value: CoinAmount) -> Result<Self, Self::Error> {
        match value {
            CoinAmount::Sats(sats) => Ok(Self::from_sat(sats)),
            CoinAmount::Btc(btc) => Self::from_btc(btc),
        }
    }
}

impl From<Amount> for CoinAmount {
    fn from(value: Amount) -> Self {
        Self::Sats(value.to_sat())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denominations_are_explicit_and_bitcoin_precision_is_checked() {
        for (json, sats) in [(r#"{"Sats":1}"#, 1), (r#"{"Btc":0.00000001}"#, 1)] {
            let amount: CoinAmount = serde_json::from_str(json).unwrap();
            assert_eq!(Amount::try_from(amount).unwrap().to_sat(), sats);
            assert_eq!(
                serde_json::to_value(amount).unwrap(),
                serde_json::from_str::<serde_json::Value>(json).unwrap()
            );
        }
        for btc in [-1.0, 0.000000001, f64::NAN, f64::INFINITY] {
            assert!(Amount::try_from(CoinAmount::Btc(btc)).is_err());
        }
    }
}
