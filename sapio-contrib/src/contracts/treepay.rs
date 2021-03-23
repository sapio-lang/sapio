// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! contracts for paying a large set of recipients fee efficiently
use sapio::contract::*;
use sapio::*;
use schemars::*;
use serde::*;
use std::convert::TryInto;
/// instructions to send an amount of coin to an address
#[derive(JsonSchema, Serialize, Deserialize, Clone)]
pub struct Payment {
    /// The amount of coin to send
    pub amount: bitcoin::util::amount::CoinAmount,
    /// # Address
    /// The Address to send to
    pub address: bitcoin::Address,
}
/// Create a tree of payments with a given radix
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct TreePay {
    /// the list of payments to create
    pub participants: Vec<Payment>,
    /// the radix to use (4 or 5 near optimal, depending on if CTV emulation is used this may be inaccurate)
    pub radix: usize,
}

impl TreePay {
    then! {expand |s, ctx| {
        let mut builder = ctx.template();
        if s.participants.len() > s.radix {

            for c in s.participants.chunks(s.participants.len()/s.radix) {
                let mut amt =  bitcoin::util::amount::Amount::from_sat(0);
                for Payment{amount, ..}  in c {
                    amt += amount.clone().try_into()?;
                }
                builder = builder.add_output(amt, &TreePay {participants: c.to_vec(), radix: s.radix}, None)?;
            }
        } else {
            for Payment{amount, address} in s.participants.iter() {
                builder = builder.add_output((*amount).try_into()?, &Compiled::from_address(address.clone(), None), None)?;
            }
        }
        builder.into()
    }}
}

impl Contract for TreePay {
    declare! {then, Self::expand}
    declare! {non updatable}
}
