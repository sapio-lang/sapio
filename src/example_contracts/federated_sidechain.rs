use crate::clause::Clause;
use crate::contract::macros::*;
use crate::contract::*;
use crate::*;
use bitcoin::util::amount::CoinAmount;
use schemars::*;
use serde::*;
use std::marker::PhantomData;

#[derive(JsonSchema, Deserialize, Default)]
pub struct CanBeginRecovery;
#[derive(JsonSchema, Deserialize, Default)]
pub struct CanFinishRecovery;

pub trait RecoveryState {}

impl RecoveryState for CanFinishRecovery {}
impl RecoveryState for CanBeginRecovery {}

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

trait StateDependentActions
where
    Self: Sized,
{
    /* Should only be defined when RecoveryState is in CanFinishRecovery */
    guard! {finish_recovery}
    /* Should only be defined when RecoveryState is in CanBeginRecovery */
    then! {begin_recovery}
}
impl StateDependentActions for FederatedPegIn<CanBeginRecovery> {
    then! {begin_recovery [Self::recovery_signed] |s| {
        let mut builder = template::Builder::new().add_output(template::Output::new(
            s.amount,
            FederatedPegIn::<CanFinishRecovery> {
                keys: s.keys.clone(),
                thresh_normal: s.thresh_normal,
                keys_recovery: s.keys_recovery.clone(),
                thresh_recovery: s.thresh_recovery,
                amount: s.amount,
                _pd: PhantomData::default()
            },
            None
        )?);
        Ok(Box::new(std::iter::once(builder.into())))
    }}
}
impl StateDependentActions for FederatedPegIn<CanFinishRecovery> {
    guard! {finish_recovery |s| {
        Clause::And(vec![Clause::Older(4725 /* 4 weeks? */), Clause::Threshold(s.thresh_recovery, s.keys_recovery.iter().cloned().map(Clause::Key).collect())])
    }}
}

use std::convert::TryInto;
impl<T: RecoveryState> FederatedPegIn<T> {
    guard! {recovery_signed |s| {
        Clause::Threshold(s.thresh_recovery, s.keys_recovery.iter().cloned().map(Clause::Key).collect())
    }}

    guard! {normal_signed |s| {
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

pub type PegIn = FederatedPegIn<CanBeginRecovery>;
