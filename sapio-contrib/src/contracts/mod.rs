use bitcoin::util::amount::CoinAmount;
use sapio::clause::Clause;

use sapio::contract::*;
use sapio::*;
use schemars::*;
use serde::*;
use std::convert::TryInto;
pub mod derivatives;
pub mod dynamic;
pub mod federated_sidechain;
pub mod hodl_chicken;
pub mod readme_contracts;
pub mod treepay;
pub mod undo_send;
pub mod vault;

#[derive(JsonSchema, Serialize, Deserialize)]
pub struct ExampleA {
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    amount: CoinAmount,
    resolution: Compiled,
}

impl ExampleA {
    guard!(timeout | s, ctx | { Clause::Older(100) });
    guard!(cached signed |s, ctx| {Clause::And(vec![Clause::Key(s.alice), Clause::Key(s.bob)])});
}

impl Contract for ExampleA {
    declare! {finish, Self::signed, Self::timeout}
    declare! {non updatable}
}

use std::marker::PhantomData;
pub trait BState: JsonSchema {
    fn get_n(_n: u8, max: u8) -> u8 {
        return max;
    }
}
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct Start;
impl BState for Start {}
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct Finish;
impl BState for Finish {
    fn get_n(n: u8, _max: u8) -> u8 {
        return n;
    }
}

pub trait ExampleBThen
where
    Self: Sized,
{
    then! {begin_contest}
}

#[derive(JsonSchema, Serialize, Deserialize)]
pub struct ExampleB<T: BState> {
    participants: Vec<bitcoin::PublicKey>,
    threshold: u8,
    amount: CoinAmount,
    #[serde(skip)]
    pd: PhantomData<T>,
}

impl<T: BState> ExampleB<T> {
    guard!(timeout | s, ctx | { Clause::Older(100) });
    guard!(cached all_signed |s, ctx| {Clause::Threshold(T::get_n(s.threshold, s.participants.len()as u8) as usize, s.participants.iter().map(|k| Clause::Key(*k)).collect())});
}

impl ExampleBThen for ExampleB<Finish> {}
impl ExampleBThen for ExampleB<Start> {
    then! {begin_contest |s, ctx| {
        ctx.template().add_output(
            s.amount.try_into()?,
            &ExampleB::<Finish> {
                participants: s.participants.clone(),
                threshold: s.threshold,
                amount: s.amount,
                pd: Default::default(),
            },
            None,
        )?.into()
    }}
}

impl<T: BState> Contract for ExampleB<T>
where
    ExampleB<T>: ExampleBThen + 'static,
{
    declare! {then, Self::begin_contest}
    declare! {finish, Self::all_signed}
    declare! {non updatable }
}
