//! HODL Chicken is a fun game to see who has stronger hands.
//!
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
use bitcoin::util::amount::Amount;
use sapio::contract::*;
use sapio::*;
use sapio_base::Clause;
use schemars::*;
use serde::*;
use std::convert::TryFrom;

/// Payout can be into any Compiled object
pub type Payout = Compiled;
#[derive(JsonSchema, Serialize, Deserialize, Clone)]
struct Payouts {
    /// Winner
    winner: Payout,
    /// Loser
    loser: Payout,
}
/// The `HodlChickenInner` has been structurally verified
/// during conversion from `HodlChickenChecks`
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

/// A wrapper around HodlChickenInner that ensures
/// invariants on values are kept.
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
    guard! {fn alice_is_a_chicken(self, _ctx) {Clause::Key(self.alice_key)}}
    guard! {fn bob_is_a_chicken(self, _ctx) {Clause::Key(self.bob_key)}}
    then! {
        guarded_by: [Self::alice_is_a_chicken]
        fn alice_redeem(self, ctx) {
        ctx.template()
            .add_output(
                Amount::from_sat(self.winner_gets),
                &self.bob_contract.winner,
                None,
            )?
            .add_output(
                Amount::from_sat(self.chicken_gets),
                &self.alice_contract.loser,
                None,
            )?
            .into()
    }}

    then! {
        guarded_by: [Self::bob_is_a_chicken]
        fn bob_redeem(self, ctx) {
            ctx.template()
                .add_output(
                    Amount::from_sat(self.winner_gets),
                    &self.alice_contract.winner,
                    None,
                )?
                .add_output(
                    Amount::from_sat(self.chicken_gets),
                    &self.bob_contract.loser,
                    None,
                )?
                .into()
        }
    }
}

impl Contract for HodlChickenInner {
    declare! {then, Self::alice_redeem, Self::bob_redeem}
    declare! {non updatable}
}
