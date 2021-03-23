// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[deny(missing_docs)]
use sapio::contract::*;
use sapio::*;
use sapio_wasm_plugin::client::*;
use sapio_wasm_plugin::*;
use schemars::*;
use serde::*;

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
/// Documentation placed here will be visible to users!
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct TreePay {
    /// all of the payments needing to be sent
    pub participants: Vec<Payment>,
    /// the radix of the tree to build. Optimal for users should be around 4 or
    /// 5 (with CTV, not emulators).
    pub radix: usize,
}

impl TreePay {
    then! {expand |s, ctx| {
        let mut builder = ctx.template();
        if s.participants.len() > s.radix {

            for c in s.participants.chunks(s.participants.len()/s.radix) {
                let mut amt =  bitcoin::util::amount::Amount::from_sat(0);
                for Payment{amount, ..}  in c {
                    amt += *amount;
                }
                builder = builder.add_output(amt, &TreePay {participants: c.to_vec(), radix: s.radix}, None)?;
            }
        } else {
            for Payment{amount, address} in s.participants.iter() {
                builder = builder.add_output(*amount, &Compiled::from_address(address.clone(), None), None)?;
            }
        }
        builder.into()
    }}
}
impl Contract for TreePay {
    declare! {then, Self::expand}
    declare! {non updatable}
}
REGISTER![TreePay];
