// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Contracts from the sapio README.md
use bitcoin::util::amount::CoinAmount;
use sapio::contract::*;
use sapio::*;
use sapio_base::timelocks::RelTime;
use sapio_base::Clause;
use sapio_macros::guard;
use schemars::*;
use serde::*;
use std::convert::TryFrom;
use std::convert::TryInto;

/// Pay To Public Key Sapio Contract
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct PayToPublicKey {
    #[schemars(with = "String")]
    key: bitcoin::XOnlyPublicKey,
}

impl PayToPublicKey {
    #[guard]
    fn with_key(self, _ctx: Context) {
        Clause::Key(self.key)
    }
}

impl Contract for PayToPublicKey {
    declare! {finish, Self::with_key}
    declare! {non updatable}
}

/// Basic Escrowing Contract
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct BasicEscrow {
    #[schemars(with = "String")]
    alice: bitcoin::XOnlyPublicKey,
    #[schemars(with = "String")]
    bob: bitcoin::XOnlyPublicKey,
    #[schemars(with = "String")]
    escrow: bitcoin::XOnlyPublicKey,
}

impl BasicEscrow {
    #[guard]
    fn redeem(self, _ctx: Context) {
        Clause::Threshold(
            1,
            vec![
                Clause::Threshold(2, vec![Clause::Key(self.alice), Clause::Key(self.bob)]),
                Clause::And(vec![
                    Clause::Key(self.escrow),
                    Clause::Threshold(1, vec![Clause::Key(self.alice), Clause::Key(self.bob)]),
                ]),
            ],
        )
    }
}

impl Contract for BasicEscrow {
    declare! {finish, Self::redeem}
    declare! {non updatable}
}

/// Basic Escrowing Contract, written more expressively
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct BasicEscrow2 {
    #[schemars(with = "String")]
    alice: bitcoin::XOnlyPublicKey,
    #[schemars(with = "String")]
    bob: bitcoin::XOnlyPublicKey,
    #[schemars(with = "String")]
    escrow: bitcoin::XOnlyPublicKey,
}

impl BasicEscrow2 {
    #[guard]
    fn use_escrow(self, _ctx: Context) {
        Clause::And(vec![
            Clause::Key(self.escrow),
            Clause::Threshold(1, vec![Clause::Key(self.alice), Clause::Key(self.bob)]),
        ])
    }
    #[guard]
    fn cooperate(self, _ctx: Context) {
        Clause::And(vec![Clause::Key(self.alice), Clause::Key(self.bob)])
    }
}

impl Contract for BasicEscrow2 {
    declare! {finish, Self::use_escrow, Self::cooperate}
    declare! {non updatable}
}

/// Trustless Escrowing Contract
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct TrustlessEscrow {
    #[schemars(with = "String")]
    alice: bitcoin::XOnlyPublicKey,
    #[schemars(with = "String")]
    bob: bitcoin::XOnlyPublicKey,
    alice_escrow: (CoinAmount, bitcoin::Address),
    bob_escrow: (CoinAmount, bitcoin::Address),
}

impl TrustlessEscrow {
    #[guard]
    fn cooperate(self, _ctx: Context) {
        Clause::And(vec![Clause::Key(self.alice), Clause::Key(self.bob)])
    }
    #[then]
    fn use_escrow(self, ctx: sapio::Context) {
        ctx.template()
            .add_output(
                self.alice_escrow.0.try_into()?,
                &Compiled::from_address(self.alice_escrow.1.clone(), None),
                None,
            )?
            .add_output(
                self.bob_escrow.0.try_into()?,
                &Compiled::from_address(self.bob_escrow.1.clone(), None),
                None,
            )?
            .set_sequence(
                0,
                RelTime::try_from(std::time::Duration::from_secs(10 * 24 * 60 * 60))?.into(),
            )?
            .into()
    }
}

impl Contract for TrustlessEscrow {
    declare! {finish, Self::cooperate}
    declare! {then, Self::use_escrow}
    declare! {non updatable}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{address, context, key};

    #[test]
    fn key_and_both_escrow_spellings_compile_with_matching_authorization() {
        let key_contract = PayToPublicKey { key: key(1) };
        assert_eq!(key_contract.guard_with_key(context(0)), Clause::Key(key(1)));
        key_contract
            .compile(context(1000))
            .unwrap()
            .validate()
            .unwrap();
        let a = BasicEscrow {
            alice: key(1),
            bob: key(2),
            escrow: key(3),
        };
        let b = BasicEscrow2 {
            alice: key(1),
            bob: key(2),
            escrow: key(3),
        };
        a.compile(context(1000)).unwrap().validate().unwrap();
        b.compile(context(1000)).unwrap().validate().unwrap();
        assert_eq!(
            b.guard_use_escrow(context(0)),
            Clause::And(vec![
                Clause::Key(key(3)),
                Clause::Threshold(1, vec![Clause::Key(key(1)), Clause::Key(key(2))])
            ])
        );
        assert_eq!(
            b.guard_cooperate(context(0)),
            Clause::And(vec![Clause::Key(key(1)), Clause::Key(key(2))])
        );
    }

    #[test]
    fn trustless_escrow_commits_to_both_payments_after_timeout() {
        let contract = TrustlessEscrow {
            alice: key(1),
            bob: key(2),
            alice_escrow: (bitcoin::Amount::from_sat(400).into(), address(1)),
            bob_escrow: (bitcoin::Amount::from_sat(600).into(), address(2)),
        };
        let object = contract.compile(context(1000)).unwrap();
        object.validate().unwrap();
        let template = object.ctv_to_tx.values().next().unwrap();
        assert_eq!(
            template
                .tx
                .output
                .iter()
                .map(|o| o.value)
                .collect::<Vec<_>>(),
            vec![400, 600]
        );
        assert_ne!(template.tx.input[0].sequence & (1 << 22), 0);
        assert!(contract.compile(context(999)).is_err());
    }
}
