//! The Contracts from the sapio README.md
use bitcoin::util::amount::CoinAmount;
use sapio::contract::*;
use sapio::*;
use sapio_base::timelocks::RelTime;
use sapio_base::Clause;
use schemars::*;
use serde::*;
use std::convert::TryFrom;
use std::convert::TryInto;

/// Pay To Public Key Sapio Contract
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct PayToPublicKey {
    key: bitcoin::PublicKey,
}

impl PayToPublicKey {
    guard!(with_key | s, ctx | { Clause::Key(s.key) });
}

impl Contract for PayToPublicKey {
    declare! {finish, Self::with_key}
    declare! {non updatable}
}

/// Basic Escrowing Contract
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct BasicEscrow {
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    escrow: bitcoin::PublicKey,
}

impl BasicEscrow {
    guard!(
        redeem | s,
        ctx | {
            Clause::Threshold(
                1,
                vec![
                    Clause::Threshold(2, vec![Clause::Key(s.alice), Clause::Key(s.bob)]),
                    Clause::And(vec![
                        Clause::Key(s.escrow),
                        Clause::Threshold(1, vec![Clause::Key(s.alice), Clause::Key(s.bob)]),
                    ]),
                ],
            )
        }
    );
}

impl Contract for BasicEscrow {
    declare! {finish, Self::redeem}
    declare! {non updatable}
}

/// Basic Escrowing Contract, written more expressively
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct BasicEscrow2 {
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    escrow: bitcoin::PublicKey,
}

impl BasicEscrow2 {
    guard!(
        use_escrow | s,
        ctx | {
            Clause::And(vec![
                Clause::Key(s.escrow),
                Clause::Threshold(2, vec![Clause::Key(s.alice), Clause::Key(s.bob)]),
            ])
        }
    );
    guard!(
        cooperate | s,
        ctx | { Clause::And(vec![Clause::Key(s.alice), Clause::Key(s.bob)]) }
    );
}

impl Contract for BasicEscrow2 {
    declare! {finish, Self::use_escrow, Self::cooperate}
    declare! {non updatable}
}

/// Trustless Escrowing Contract
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct TrustlessEscrow {
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    alice_escrow: (CoinAmount, bitcoin::Address),
    bob_escrow: (CoinAmount, bitcoin::Address),
}

impl TrustlessEscrow {
    guard!(
        cooperate | s,
        ctx | { Clause::And(vec![Clause::Key(s.alice), Clause::Key(s.bob)]) }
    );
    then! {use_escrow |s, ctx| {
        ctx.template()
            .add_output(
                s.alice_escrow.0.try_into()?,
                &Compiled::from_address(s.alice_escrow.1.clone(), None),
                None)?
            .add_output(
                s.bob_escrow.0.try_into()?,
                &Compiled::from_address(s.bob_escrow.1.clone(), None),
                None)?
            .set_sequence(0, RelTime::try_from(std::time::Duration::from_secs(10*24*60*60))?.into())?.into()
    }}
}

impl Contract for TrustlessEscrow {
    declare! {finish, Self::cooperate}
    declare! {then, Self::use_escrow}
    declare! {non updatable}
}
