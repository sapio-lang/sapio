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
use sapio_macros::guard;
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
pub struct HodlChickenInner(HodlChickenChecks);

/// Unchecked wire representation, validated before constructing the contract.
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct HodlChickenChecks {
    alice_contract: Payouts,
    bob_contract: Payouts,
    #[schemars(with = "String")]
    alice_key: bitcoin::XOnlyPublicKey,
    #[schemars(with = "String")]
    bob_key: bitcoin::XOnlyPublicKey,
    alice_deposit: u64,
    bob_deposit: u64,
    winner_gets: u64,
    chicken_gets: u64,
}

impl TryFrom<HodlChickenChecks> for HodlChickenInner {
    type Error = &'static str;
    fn try_from(a: HodlChickenChecks) -> Result<Self, Self::Error> {
        let inner = a;
        let deposits = inner.alice_deposit.checked_add(inner.bob_deposit);
        let outputs = inner.winner_gets.checked_add(inner.chicken_gets);
        if deposits != outputs {
            Err("Outputs not Equal Deposits")
        } else if deposits.is_none() {
            Err("Amounts Overflow")
        } else if inner.alice_deposit != inner.bob_deposit {
            Err("Amounts differ")
        } else {
            Ok(Self(inner))
        }
    }
}

impl HodlChickenInner {
    #[guard]
    fn alice_is_a_chicken(self, _ctx: Context) {
        Clause::Key(self.0.alice_key)
    }
    #[guard]
    fn bob_is_a_chicken(self, _ctx: Context) {
        Clause::Key(self.0.bob_key)
    }
    #[then(guarded_by = "[Self::alice_is_a_chicken]")]
    fn alice_redeem(self, ctx: sapio::Context) {
        ctx.template()
            .add_output(
                Amount::from_sat(self.0.winner_gets),
                &self.0.bob_contract.winner,
                None,
            )?
            .add_output(
                Amount::from_sat(self.0.chicken_gets),
                &self.0.alice_contract.loser,
                None,
            )?
            .into()
    }

    #[then(guarded_by = "[Self::bob_is_a_chicken]")]
    fn bob_redeem(self, ctx: sapio::Context) {
        ctx.template()
            .add_output(
                Amount::from_sat(self.0.winner_gets),
                &self.0.alice_contract.winner,
                None,
            )?
            .add_output(
                Amount::from_sat(self.0.chicken_gets),
                &self.0.bob_contract.loser,
                None,
            )?
            .into()
    }
}

impl Contract for HodlChickenInner {
    fn ensure_amount(&self, _ctx: Context) -> Result<Amount, CompilationError> {
        Ok(Amount::from_sat(self.0.alice_deposit + self.0.bob_deposit))
    }
    declare! {then, Self::alice_redeem, Self::bob_redeem}
    declare! {non updatable}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{address, context, key};

    fn input() -> serde_json::Value {
        serde_json::json!({
            "alice_contract": {"winner": Compiled::from_address(address(3), None),
                               "loser": Compiled::from_address(address(4), None)},
            "bob_contract": {"winner": Compiled::from_address(address(5), None),
                             "loser": Compiled::from_address(address(6), None)},
            "alice_key": key(1), "bob_key": key(2),
            "alice_deposit": 1000, "bob_deposit": 1000,
            "winner_gets": 1500, "chicken_gets": 500
        })
    }

    #[test]
    fn flat_json_round_trip_and_both_chicken_payouts() {
        let json = input();
        let game: HodlChickenInner = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(&game).unwrap(), json);
        let compiled = game.compile(context(2000)).unwrap();
        assert_eq!(compiled.ctv_to_tx.len(), 2);
        let mut payouts: Vec<_> = compiled
            .ctv_to_tx
            .values()
            .map(|t| {
                assert_eq!(t.tx.input.len(), 1);
                assert_eq!(
                    t.tx.output.iter().map(|o| o.value).collect::<Vec<_>>(),
                    vec![1500, 500]
                );
                (
                    t.tx.output[0].script_pubkey.clone(),
                    t.tx.output[1].script_pubkey.clone(),
                )
            })
            .collect();
        payouts.sort();
        let mut expected = vec![
            (address(5).script_pubkey(), address(4).script_pubkey()),
            (address(3).script_pubkey(), address(6).script_pubkey()),
        ];
        expected.sort();
        assert_eq!(payouts, expected);
        assert!(game.compile(context(1999)).is_err());
    }

    #[test]
    fn invalid_deposits_outputs_and_overflows_are_rejected() {
        for (alice, bob, winner, chicken) in [
            (999, 1001, 1500, 500),
            (1000, 1000, 1500, 501),
            (u64::MAX, 1, 1500, 500),
            (1000, 1000, u64::MAX, 1),
            (u64::MAX, u64::MAX, u64::MAX, u64::MAX),
        ] {
            let mut json = input();
            json["alice_deposit"] = alice.into();
            json["bob_deposit"] = bob.into();
            json["winner_gets"] = winner.into();
            json["chicken_gets"] = chicken.into();
            assert!(serde_json::from_value::<HodlChickenInner>(json).is_err());
        }
        let mut json = input();
        for field in [
            "alice_deposit",
            "bob_deposit",
            "winner_gets",
            "chicken_gets",
        ] {
            json[field] = (u64::MAX / 2).into();
        }
        assert!(serde_json::from_value::<HodlChickenInner>(json).is_ok());
        let schema = serde_json::to_value(schemars::schema_for!(HodlChickenInner)).unwrap();
        assert!(schema.to_string().contains("alice_deposit"));
    }
}
