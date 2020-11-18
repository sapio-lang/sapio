use bitcoin::util::amount::CoinAmount;
use sapio::clause::Clause;
use sapio::contract::macros::*;
use sapio::contract::*;
use sapio::*;
use schemars::*;
use serde::*;

/// Pay To Public Key Sapio Contract
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct PayToPublicKey {
    key: bitcoin::PublicKey,
}

impl<'a> PayToPublicKey {
    guard!(with_key | s | { Clause::Key(s.key) });
}

impl<'a> Contract<'a> for PayToPublicKey {
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

impl<'a> BasicEscrow {
    guard!(
        redeem | s | {
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

impl<'a> Contract<'a> for BasicEscrow {
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

impl<'a> BasicEscrow2 {
    guard!(
        use_escrow | s | {
            Clause::And(vec![
                Clause::Key(s.escrow),
                Clause::Threshold(2, vec![Clause::Key(s.alice), Clause::Key(s.bob)]),
            ])
        }
    );
    guard!(cooperate | s | { Clause::And(vec![Clause::Key(s.alice), Clause::Key(s.bob)]) });
}

impl<'a> Contract<'a> for BasicEscrow2 {
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

impl<'a> TrustlessEscrow {
    guard!(cooperate | s | { Clause::And(vec![Clause::Key(s.alice), Clause::Key(s.bob)]) });
    then! {use_escrow |s| {
        let o1 = txn::Output::new(
            s.alice_escrow.0,
            Compiled::from_address(s.alice_escrow.1.clone(), None),
            None,
        )?;
        let o2 = txn::Output::new(
            s.bob_escrow.0,
            Compiled::from_address(s.bob_escrow.1.clone(), None),
            None,
        )?;
        let mut tb = txn::TemplateBuilder::new().add_output(o1).add_output(o2).set_sequence(0, 1700 /*roughly 10 days*/);
        Ok(Box::new(std::iter::once(
            tb.into(),
        )))
    }}
}

impl<'a> Contract<'a> for TrustlessEscrow {
    declare! {finish, Self::cooperate}
    declare! {then, Self::use_escrow}
    declare! {non updatable}
}
