// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A Contract that offers peg-in functionality for sidechains
use bitcoin::util::amount::CoinAmount;
use sapio::contract::*;
use sapio::*;
use sapio_base::Clause;
use schemars::*;
use serde::*;
use std::convert::TryInto;
use std::marker::PhantomData;

/// State  when recover may start
#[derive(JsonSchema, Deserialize, Default)]
pub struct CanBeginRecovery;
/// State when recovery may complete
#[derive(JsonSchema, Deserialize, Default)]
pub struct CanFinishRecovery;

/// trait-level enum for states of a FederatedPegIn
pub trait RecoveryState {}

impl RecoveryState for CanFinishRecovery {}
impl RecoveryState for CanBeginRecovery {}

/// A contract for depositing into a federated side chain.
#[derive(JsonSchema, Deserialize)]
pub struct FederatedPegIn<T: RecoveryState> {
    keys: Vec<bitcoin::PublicKey>,
    thresh_normal: usize,
    keys_recovery: Vec<bitcoin::PublicKey>,
    thresh_recovery: usize,
    amount: CoinAmount,
    #[serde(skip)]
    _pd: PhantomData<T>,
}

/// Actions that will be specialized depending on the exact state.
pub trait StateDependentActions
where
    Self: Sized,
{
    guard! {
    /// Should only be defined when RecoveryState is in CanFinishRecovery
    finish_recovery}

    then! {
    /// Should only be defined when RecoveryState is in CanBeginRecovery
    begin_recovery}
}
impl StateDependentActions for FederatedPegIn<CanBeginRecovery> {
    then! {begin_recovery [Self::recovery_signed] |s, ctx| {
        ctx.template().add_output(
            s.amount.try_into()?,
            &FederatedPegIn::<CanFinishRecovery> {
                keys: s.keys.clone(),
                thresh_normal: s.thresh_normal,
                keys_recovery: s.keys_recovery.clone(),
                thresh_recovery: s.thresh_recovery,
                amount: s.amount,
                _pd: PhantomData::default()
            },
            None
        )?.into()
    }}
}
impl StateDependentActions for FederatedPegIn<CanFinishRecovery> {
    guard! {finish_recovery |s, ctx| {
        Clause::And(vec![Clause::Older(4725 /* 4 weeks? */), Clause::Threshold(s.thresh_recovery, s.keys_recovery.iter().cloned().map(Clause::Key).collect())])
    }}
}

impl<T: RecoveryState> FederatedPegIn<T> {
    guard! {recovery_signed |s, ctx| {
        Clause::Threshold(s.thresh_recovery, s.keys_recovery.iter().cloned().map(Clause::Key).collect())
    }}

    guard! {normal_signed |s, ctx| {
        Clause::Threshold(s.thresh_normal, s.keys.iter().cloned().map(Clause::Key).collect())
    }}
}

impl<T: RecoveryState> Contract for FederatedPegIn<T>
where
    FederatedPegIn<T>: StateDependentActions + 'static,
{
    declare! {then, Self::begin_recovery}
    declare! {finish, Self::normal_signed, Self::finish_recovery}
    declare! {non updatable}
}

/// Type Alias for the state to start FederatedPegIn from.
pub type PegIn = FederatedPegIn<CanBeginRecovery>;
