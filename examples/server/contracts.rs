use bitcoin::util::amount::CoinAmount;
use sapio::clause::Clause;
use sapio::contract::macros::*;
use sapio::contract::*;
use sapio::*;
use schemars::*;
use serde::*;

#[derive(JsonSchema, Serialize, Deserialize)]
pub struct ExampleA {
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    amount: CoinAmount,
    resolution: Compiled,
}

pub struct Args;
impl<'a> ExampleA {
    guard!(timeout | s | { Clause::Older(100) });
    guard!(cached signed |s| {Clause::And(vec![Clause::Key(s.alice), Clause::Key(s.bob)])});
}

impl<'a> Contract<'a> for ExampleA {
    def! {finish, Self::signed, Self::timeout}
    def! {updatable<Args> }
}

use std::marker::PhantomData;
pub trait BState : JsonSchema  {
    fn get_n(n: u8, max: u8) -> u8 {
        return max;
    }
}
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct Start;
impl BState for Start {}
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct Finish;
impl BState for Finish {
    fn get_n(n: u8, max: u8) -> u8 {
        return n;
    }
}
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct ExampleB<T: BState> {
    participants: Vec<bitcoin::PublicKey>,
    threshold: u8,
    amount: CoinAmount,
    pd: PhantomData<T>,
}

impl<'a, T: BState> ExampleB<T> {
    guard!(timeout | s | { Clause::Older(100) });
    guard!(cached all_signed |s| {Clause::Threshold(T::get_n(s.threshold, s.participants.len()as u8) as usize, s.participants.iter().map(|k| Clause::Key(*k)).collect())});

    then! {begin_contest |s| {
        let o = txn::Output::new(
            s.amount,
            ExampleB::<Finish> {
                participants: s.participants.clone(),
                threshold: s.threshold,
                amount: s.amount,
                pd: Default::default(),
            },
            None,
        )?;
        Ok(Box::new(std::iter::once(
            txn::TemplateBuilder::new().add_output(o).into(),
        )))
    }}
}

impl<'a, T: BState + 'a> Contract<'a> for ExampleB<T> {
    def! {finish, Self::all_signed}
    def! {updatable<Args> }
}
