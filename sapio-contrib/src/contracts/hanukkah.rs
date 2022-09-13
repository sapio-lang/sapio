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
    /// feerate
    #[serde(with = "bitcoin::util::amount::serde::as_sat")]
    #[schemars(with = "u64")]
    feerate_per_byte: Amount,
    /// What time should the Hanukkiah be able to be lit the first night, subsequent nights will be 24 hours later.
    night_time: AbsTime,
    #[serde(skip)]
    night: Option<u8>,
}

fn candles_left(s: u8) -> u8 {
    if s == 8 {
        0
    } else {
        s + candles_left(s + 1)
    }
}
impl Hanukkiah {
    #[then]
    fn light_candles(self, ictx: Context) {
        let mut ctx = ictx;
        let mut txn = ctx.derive_num(0u64)?.template();
        let night = self.night.unwrap_or(1);
        if night < 8 {
            let next_night = ctx.derive_num(1u64)?.compile(Hanukkiah {
                night: Some(night + 1),
                ..self.clone()
            })?;
            txn = txn.add_output(next_night.amount_range.max(), &next_night, None)?;
        }
        for _ in 0..night {
            txn = txn.add_output(
                self.amount_per_candle.into(),
                &Compiled::from_address(self.recipient.clone(), None),
                None,
            )?;
        }
        let size = txn.estimate_tx_size();
        txn = txn.add_amount(self.feerate_per_byte * size);
        let candle_time =
            AbsTime::try_from(self.night_time.get() + 24 * 60 * 60 * (night as u32 - 1_u32))?
                .into();
        txn.set_lock_time(candle_time)?.into()
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
    /// feerate
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
    /// feerate
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
        let mut txn = ctx.template();
        let mut r = self.recipients.clone();
        for _ in 0..self.night {
            txn = txn.add_output(
                self.amount_per_candle.into(),
                &Compiled::from_address(
                    r.pop().ok_or(CompilationError::TerminateCompilation)?,
                    None,
                ),
                None,
            )?;
        }
        let size = txn.estimate_tx_size();
        let fees = self.feerate_per_byte * size;
        txn = txn.add_amount(fees);
        txn = txn.add_fees(fees)?;
        let candle_time =
            AbsTime::try_from(self.night_time.get() + 24 * 60 * 60 * (self.night as u32 - 1))?
                .into();
        txn.set_lock_time(candle_time)?.into()
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
            txn = txn.add_output(next_night.amount_range.max(), &next_night, None)?;
        }
        let size = txn.estimate_tx_size();
        let fees = self.feerate_per_byte * size;
        txn = txn.add_amount(fees);
        txn = txn.add_fees(fees)?;
        txn.into()
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
