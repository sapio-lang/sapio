use crate::clause::Clause;
use crate::contract::macros::*;
use crate::contract::*;
use crate::*;
/**
* This License applies solely to the file hodl_chicken.rs.
* Copyright (c) 2020, Pyskell and Judica, Inc
* All rights reserved.
* Redistribution and use in source and binary forms, with or without
* modification, are permitted provided that the following conditions are met:
*     * Redistributions of source code must retain the above copyright
*       notice, this list of conditions and the following disclaimer.
*     * Redistributions in binary form must reproduce the above copyright
*       notice, this list of conditions and the following disclaimer in the
*       documentation and/or other materials provided with the distribution.
*     * Neither the name of the <organization> nor the
*       names of its contributors may be used to endorse or promote products
*       derived from this software without specific prior written permission.
* THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
* ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
* WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
* DISCLAIMED. IN NO EVENT SHALL <COPYRIGHT HOLDER> BE LIABLE FOR ANY
* DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
* (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
* LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
* ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
* (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
* SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
**/
use bitcoin::util::amount::CoinAmount;
use schemars::*;
use serde::*;
use std::collections::HashMap;
use std::convert::TryFrom;

pub type Payout = Compiled;
#[derive(JsonSchema, Serialize, Deserialize, Clone)]
struct Payouts {
    /// Winner
    winner: Payout,
    /// Loser
    loser: Payout,
}
#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(try_from = "HodlChickenChecks")]
pub struct HodlChickenInner {
    alice_contract: Payouts,
    bob_contract: Payouts,
    alice_key: bitcoin::PublicKey,
    bob_key: bitcoin::PublicKey,
    alice_deposit: u64,
    bob_deposit: u64,
    winner_gets: u64,
    chicken_gets: u64,
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HodlChickenChecks(HodlChickenInner);

impl TryFrom<HodlChickenChecks> for HodlChickenInner {
    type Error = &'static str;
    fn try_from(a: HodlChickenChecks) -> Result<Self, Self::Error> {
        let inner = a.0;
        let deposits = inner.alice_deposit.checked_add(inner.bob_deposit);
        let outputs = inner.winner_gets.checked_add(inner.chicken_gets);
        if deposits != outputs {
            Err("Outputs not Equal Deposits")
        } else if deposits == None {
            Err("Amounts Overflow")
        } else if inner.alice_deposit != inner.bob_deposit {
            Err("Amounts differ")
        } else {
            Ok(inner)
        }
    }
}

impl HodlChickenInner {
    guard! {alice_is_a_chicken |s| {Clause::Key(s.alice_key)}}
    guard! {bob_is_a_chicken |s| {Clause::Key(s.bob_key)}}
    then! {alice_redeem [Self::alice_is_a_chicken] |s| {
        Ok(Box::new(std::iter::once(Ok(txn::TemplateBuilder::new()
            .add_output(txn::Output::new(
                CoinAmount::Sats(s.winner_gets),
                s.bob_contract.winner.clone(),
                None,
            )?)
            .add_output(txn::Output::new(
                CoinAmount::Sats(s.chicken_gets),
                s.alice_contract.loser.clone(),
                None,
            )?)
            .into()))))
    }}

    then! {bob_redeem [Self::bob_is_a_chicken] |s| {
        Ok(Box::new(std::iter::once(Ok(txn::TemplateBuilder::new()
            .add_output(txn::Output::new(
                CoinAmount::Sats(s.winner_gets),
                s.alice_contract.winner.clone(),
                None,
            )?)
            .add_output(txn::Output::new(
                CoinAmount::Sats(s.chicken_gets),
                s.bob_contract.loser.clone(),
                None,
            )?)
            .into()))))
    }}
}

impl Contract for HodlChickenInner {
    declare! {then, Self::alice_redeem, Self::bob_redeem}
    declare! {non updatable}
}
