// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! A Hanukkah Miracle!
use bitcoin::util::amount::Amount;
use sapio::contract::*;
use sapio::util::amountrange::AmountF64;
use sapio::*;
use sapio_base::timelocks::AbsTime;

use schemars::*;
use serde::*;
use std::convert::TryFrom;

/// Implements a Hanukkiah for @TheBitcoinRabbi
#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub struct Hanukkiah {
    /// Who receives the funds in the candles
    recipient: bitcoin::Address,
    /// Amount of Coin per Candle
    amount_per_candle: AmountF64,
    /// Satoshis per unsigned transaction byte; witness costs are not included.
    #[serde(with = "bitcoin::util::amount::serde::as_sat")]
    #[schemars(with = "u64")]
    feerate_per_byte: Amount,
    /// What time should the Hanukkiah be able to be lit the first night, subsequent nights will be 24 hours later.
    night_time: AbsTime,
    #[serde(skip)]
    night: Option<u8>,
}

fn candle_time(start: AbsTime, night: u8) -> Result<AbsTime, CompilationError> {
    if !(1..=8).contains(&night) {
        return Err(CompilationError::Custom(
            "Candle night must be between 1 and 8".into(),
        ));
    }
    let time = start
        .get()
        .checked_add(24 * 60 * 60 * u32::from(night - 1))
        .ok_or(CompilationError::TerminateCompilation)?;
    Ok(AbsTime::try_from(time)?)
}
impl Hanukkiah {
    #[then]
    fn light_candles(self, ictx: Context) {
        let mut ctx = ictx;
        let mut txn = ctx.derive_num(0u64)?.template();
        let night = self.night.unwrap_or(1);
        let lock_time = candle_time(self.night_time, night)?;
        if night < 8 {
            let next_night = ctx.derive_num(1u64)?.compile(Hanukkiah {
                night: Some(night + 1),
                ..self.clone()
            })?;
            txn = txn.add_output(next_night.required_input_amount, &next_night, None)?;
        }
        for _ in 0..night {
            txn = txn.add_output(
                self.amount_per_candle.into(),
                &Compiled::from_address(self.recipient.clone(), Amount::ZERO),
                None,
            )?;
        }
        let size = txn.unsigned_tx_size();
        let fees = self
            .feerate_per_byte
            .checked_mul(size)
            .ok_or(CompilationError::OutOfFunds)?;
        txn.set_lock_time(lock_time.into())?.add_fees(fees)?.into()
    }
}
impl Contract for Hanukkiah {
    declare! {then, Self::light_candles}
    declare! {non updatable}
}

/// Implements a Hanukkiah for @TheBitcoinRabbi
/// Fat Version
#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub struct Hanukkiah2 {
    /// Who receives the funds in the candles
    recipient: Recipients,
    /// Amount of Coin per Candle
    amount_per_candle: AmountF64,
    /// Satoshis per unsigned transaction byte; witness costs are not included.
    #[serde(with = "bitcoin::util::amount::serde::as_sat")]
    #[schemars(with = "u64")]
    feerate_per_byte: Amount,
    /// What time should the Hanukkiah be able to be lit the first night, subsequent nights will be 24 hours later.
    night_time: AbsTime,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
#[serde(try_from = "String")]
#[serde(into = "String")]
#[schemars(transparent)]
struct Recipients(#[schemars(with = "String")] [bitcoin::Address; 36]);

use std::convert::TryInto;
use std::str::FromStr;
impl TryFrom<String> for Recipients {
    type Error = CompilationError;
    fn try_from(s: String) -> std::result::Result<Self, Self::Error> {
        let v: [bitcoin::Address; 36] = s
            .split_whitespace()
            .map(bitcoin::Address::from_str)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_e| CompilationError::TerminateCompilation)?
            .try_into()
            .map_err(|_e| CompilationError::TerminateCompilation)?;
        Ok(Recipients(v))
    }
}
impl Into<String> for Recipients {
    fn into(self) -> String {
        self.0
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
struct Hanukkiah2Night {
    /// Who receives the funds in the candles
    recipients: Vec<bitcoin::Address>,
    /// Amount of Coin per Candle
    amount_per_candle: AmountF64,
    /// Satoshis per unsigned transaction byte; witness costs are not included.
    #[serde(with = "bitcoin::util::amount::serde::as_sat")]
    #[schemars(with = "u64")]
    feerate_per_byte: Amount,
    /// What time should the Hanukkiah be able to be lit the first night, subsequent nights will be 24 hours later.
    night_time: AbsTime,
    night: u8,
}

impl Hanukkiah2Night {
    #[then]
    fn light_candles(self, ctx: Context) {
        let lock_time = candle_time(self.night_time, self.night)?;
        if self.recipients.len() != usize::from(self.night) {
            return Err(CompilationError::Custom(
                "Each candle requires one recipient".into(),
            ));
        }
        let mut txn = ctx.template();
        let mut r = self.recipients.clone();
        for _ in 0..self.night {
            txn = txn.add_output(
                self.amount_per_candle.into(),
                &Compiled::from_address(
                    r.pop().ok_or(CompilationError::TerminateCompilation)?,
                    Amount::ZERO,
                ),
                None,
            )?;
        }
        let size = txn.unsigned_tx_size();
        let fees = self
            .feerate_per_byte
            .checked_mul(size)
            .ok_or(CompilationError::OutOfFunds)?;
        txn.set_lock_time(lock_time.into())?.add_fees(fees)?.into()
    }
}
impl Hanukkiah2 {
    #[then]
    fn create(self, ictx: Context) {
        let mut ctx = ictx;
        let mut txn = ctx.derive_num(0u64)?.template();
        let mut r = self.recipient.0.iter().cloned();
        for night in 1..=8 {
            let next_night = ctx
                .derive_num(night as u64 + 1u64)?
                .compile(Hanukkiah2Night {
                    night,
                    recipients: (0..night)
                        .map(|_| r.next().ok_or(CompilationError::TerminateCompilation))
                        .collect::<Result<Vec<_>, _>>()?,
                    amount_per_candle: self.amount_per_candle,
                    night_time: self.night_time,
                    feerate_per_byte: self.feerate_per_byte,
                })?;
            txn = txn.add_output(next_night.required_input_amount, &next_night, None)?;
        }
        let size = txn.unsigned_tx_size();
        let fees = self
            .feerate_per_byte
            .checked_mul(size)
            .ok_or(CompilationError::OutOfFunds)?;
        let fee_paying_txn = txn.add_fees(fees)?;
        fee_paying_txn.into()
    }
}
impl Contract for Hanukkiah2 {
    declare! {then, Self::create}
    declare! {non updatable}
}
impl Contract for Hanukkiah2Night {
    declare! {then, Self::light_candles}
    declare! {non updatable}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{address, context};

    fn assert_unsigned_fee(template: &sapio::template::Template, rate: Amount) {
        assert!(template
            .tx
            .input
            .iter()
            .all(|input| input.script_sig.is_empty() && input.witness.is_empty()));
        let bytes = bitcoin::consensus::serialize(&template.tx).len() as u64;
        assert_eq!(
            template.max - template.total_amount(),
            rate.checked_mul(bytes).unwrap()
        );
    }

    #[test]
    fn chained_candles_cover_eight_nights_and_account_for_fees() {
        let contract = Hanukkiah {
            recipient: address(1),
            amount_per_candle: Amount::from_sat(1000).into(),
            feerate_per_byte: Amount::from_sat(7),
            night_time: AbsTime::try_from(500_000_001).unwrap(),
            night: None,
        };
        let object = contract.compile(context(100_000)).unwrap();
        object.validate().unwrap();
        let mut current = &object;
        for night in 1..=8 {
            let template = current.ctv_to_tx.values().next().unwrap();
            assert_eq!(template.tx.lock_time, 500_000_001 + 86400 * (night - 1));
            assert_unsigned_fee(template, contract.feerate_per_byte);
            let candles = template
                .outputs
                .iter()
                .filter(|o| o.contract.ctv_to_tx.is_empty())
                .collect::<Vec<_>>();
            assert_eq!(candles.len(), night as usize);
            assert!(candles.iter().all(|o| o.amount.as_sat() == 1000));
            if night < 8 {
                current = &template.outputs[0].contract;
            }
        }
    }

    #[test]
    fn parallel_candles_cover_all_recipients_and_reject_bad_shapes() {
        let recipients = Recipients(std::array::from_fn(|_| address(1)));
        let text = serde_json::to_string(&recipients).unwrap();
        let parsed: Recipients = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.0.len(), 36);
        assert!(Recipients::try_from(address(1).to_string()).is_err());
        let contract = Hanukkiah2 {
            recipient: recipients,
            amount_per_candle: Amount::from_sat(1000).into(),
            feerate_per_byte: Amount::from_sat(3),
            night_time: AbsTime::try_from(500_000_001).unwrap(),
        };
        let object = contract.compile(context(100_000)).unwrap();
        object.validate().unwrap();
        let creation = object.ctv_to_tx.values().next().unwrap();
        assert_unsigned_fee(creation, contract.feerate_per_byte);
        let nights = &creation.outputs;
        assert_eq!(nights.len(), 8);
        for night in nights {
            assert_unsigned_fee(
                night.contract.ctv_to_tx.values().next().unwrap(),
                contract.feerate_per_byte,
            );
        }
        assert_eq!(
            nights
                .iter()
                .map(|o| o.contract.ctv_to_tx.values().next().unwrap().outputs.len())
                .sum::<usize>(),
            36
        );
        assert!(candle_time(contract.night_time, 0).is_err());
        assert!(candle_time(AbsTime::try_from(u32::MAX).unwrap(), 2).is_err());
        let expensive = Hanukkiah {
            recipient: address(1),
            amount_per_candle: Amount::from_sat(1000).into(),
            feerate_per_byte: Amount::from_sat(u64::MAX),
            night_time: contract.night_time,
            night: Some(8),
        };
        assert!(expensive.compile(context(100_000)).is_err());
    }
}
