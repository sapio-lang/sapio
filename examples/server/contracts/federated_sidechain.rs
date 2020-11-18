use bitcoin::util::amount::CoinAmount;
use sapio::clause::Clause;
use sapio::contract::macros::*;
use sapio::contract::*;
use sapio::*;
use schemars::*;
use serde::*;
use std::marker::PhantomData;

#[derive(JsonSchema, Deserialize, Default)]
pub struct Start;
#[derive(JsonSchema, Deserialize, Default)]
pub struct Stop;

pub trait State {}

impl State for Stop{}
impl State for Start{}

#[derive(JsonSchema, Deserialize)]
pub struct FederatedPegIn<T:State> {
    keys: Vec<bitcoin::PublicKey>,
    thresh_all: usize,
    keys_backup: Vec<bitcoin::PublicKey>,
    thresh_backup: usize,
    amount: CoinAmount,
    #[serde(skip)]
    _pd: PhantomData<T>
}

trait CloseableLo<'a> where Self : Sized {
    guard! {finish_backup}
    then! {begin_backup}
}
impl<'a> CloseableLo<'a> for FederatedPegIn<Start> {
    then! {begin_backup [Self::lo_signed] |s| {
        let mut builder = txn::TemplateBuilder::new().add_output(txn::Output::new(
            s.amount,
            FederatedPegIn::<Stop> {
                keys: s.keys.clone(),
                thresh_all: s.thresh_all,
                keys_backup: s.keys_backup.clone(),
                thresh_backup: s.thresh_backup,
                amount: s.amount,
                _pd: PhantomData::default()
            },
            None
        )?);
        Ok(Box::new(std::iter::once(builder.into())))
    }}
}
impl<'a> CloseableLo<'a> for FederatedPegIn<Stop> {
    guard!{finish_backup |s| {
        Clause::And(vec![Clause::Older(4725 /* 4 weeks? */), Clause::Threshold(s.thresh_backup, s.keys_backup.iter().cloned().map(Clause::Key).collect())])
    }}
}

use std::convert::TryInto;
impl<'a, T: State> FederatedPegIn<T> {
    guard!{lo_signed |s| {
        Clause::Threshold(s.thresh_backup, s.keys_backup.iter().cloned().map(Clause::Key).collect())
    }}

    guard!{hi_signed |s| {
        Clause::Threshold(s.thresh_all, s.keys.iter().cloned().map(Clause::Key).collect())
    }}
}

impl<'a, T:State + 'a> Contract<'a> for FederatedPegIn<T>
where FederatedPegIn<T> : CloseableLo<'a> {
    declare! {then, Self::begin_backup}
    declare! {finish, Self::hi_signed, Self::finish_backup}
    declare! {non updatable}
}


pub type PegIn = FederatedPegIn<Start>;
