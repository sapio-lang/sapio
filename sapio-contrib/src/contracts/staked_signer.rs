// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A Contract that enables a staked signing protocol
use bitcoin::XOnlyPublicKey;
use sapio::contract::*;
use sapio::*;
use sapio_base::timelocks::AnyRelTimeLock;
use sapio_base::Clause;
use sapio_macros::guard;
use schemars::*;
use serde::*;
use std::marker::PhantomData;

/// # Operational State
/// State where stakes should be recognized for voting
#[derive(JsonSchema, Deserialize)]
pub struct Operational;
/// # Closing State
/// State where stakes are closing and waiting evidence of misbehavior
#[derive(JsonSchema, Deserialize)]
struct Closing;
/// # Staking States (Operational, Closing)
/// enum trait for states
pub trait StakingState {}
impl StakingState for Operational {}
impl StakingState for Closing {}

/// # Staker: A Bonded Signing Contract
/// Staker is a contract that proceeds from Operational -> Closing
/// During it's lifetime, many things can be signed with signing_key,
/// but should the key ever leak (e.g., via nonce reuse) the bonded
/// funds can be burned.
///
/// Burning is important v.s. miner fee because otherwise the staker
/// can bribe (or be a miner themselves) to cheat.
#[derive(JsonSchema, Deserialize)]
pub struct Staker<T: StakingState> {
    /// # Timeout
    /// How long to wait for evidence after closing
    timeout: AnyRelTimeLock,
    /// # Signing Key
    /// The key that if leaked can burn funds
    // TODO: Taproot fix encoding
    #[schemars(with = "bitcoin::hashes::sha256::Hash")]
    signing_key: XOnlyPublicKey,
    /// # Redemption Key
    /// The key that will be used to control & return the redeemed funds
    // TODO: Taproot fix encoding
    #[schemars(with = "bitcoin::hashes::sha256::Hash")]
    redeeming_key: XOnlyPublicKey,
    /// current contract state.
    #[serde(skip, default)]
    state: PhantomData<T>,
}

/// Functional Interface for Staking Contracts
pub trait StakerInterface
where
    Self: Sized + Contract,
{
    decl_guard!(
        /// The key used to sign messages
        staking_key
    );
    decl_guard!(
        /// the clause to begin a close process
        begin_redeem_key
    );
    decl_guard!(
        /// the clause to finish a close process
        finish_redeem_key
    );
    decl_then!(
        /// The transition from Operational to Closing
        begin_redeem
    );

    /// WHY
    #[then(guarded_by = "[Self::staking_key]")]
    fn cheated(self, ctx: sapio::Context) {
        let f = ctx.funds();
        ctx.template()
            .add_output(f, &Compiled::from_op_return(b"dirty cheater")?, None)?
            .into()
    }
}

impl StakerInterface for Staker<Operational> {
    /// redeeming key
    #[guard]
    fn begin_redeem_key(self, _ctx: Context) {
        Clause::Key(self.redeeming_key)
    }
    /// begin redemption process
    #[then(guarded_by = "[Self::begin_redeem_key]")]
    fn begin_redeem(self, ctx: sapio::Context) {
        let f = ctx.funds();
        ctx.template()
            .add_output(
                f,
                &Staker::<Closing> {
                    state: Default::default(),
                    timeout: self.timeout,
                    signing_key: self.signing_key,
                    redeeming_key: self.redeeming_key,
                },
                None,
            )?
            .into()
    }
    /// staking key
    #[guard]
    fn staking_key(self, _ctx: Context) {
        Clause::Key(self.signing_key)
    }
}

impl StakerInterface for Staker<Closing> {
    #[guard]
    fn finish_redeem_key(self, _ctx: Context) {
        Clause::And(vec![Clause::Key(self.redeeming_key), self.timeout.into()])
    }
    #[guard]
    fn staking_key(self, _ctx: Context) {
        Clause::Key(self.signing_key)
    }
}

impl<T: 'static + StakingState> Contract for Staker<T>
where
    Staker<T>: StakerInterface,
    T: StakingState,
{
    declare! {then, Self::begin_redeem, Self::cheated}
    declare! {finish, Self::finish_redeem_key}
    declare! {non updatable}
}
