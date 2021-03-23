// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Some basic examples showing a kitchen sink of functionality
use super::*;
use std::marker::PhantomData;

#[derive(JsonSchema, Serialize, Deserialize)]
struct ExampleA {
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

trait BState: JsonSchema {
    fn get_n(_n: u8, max: u8) -> u8 {
        return max;
    }
}
#[derive(JsonSchema, Serialize, Deserialize)]
struct Start;
impl BState for Start {}
#[derive(JsonSchema, Serialize, Deserialize)]
struct Finish;
impl BState for Finish {
    fn get_n(n: u8, _max: u8) -> u8 {
        return n;
    }
}

trait ExampleBThen
where
    Self: Sized,
{
    then! {begin_contest}
}

#[derive(JsonSchema, Serialize, Deserialize)]
struct ExampleB<T: BState> {
    participants: Vec<bitcoin::PublicKey>,
    threshold: u8,
    amount: CoinAmount,
    #[serde(skip)]
    pd: PhantomData<T>,
}

impl<T: BState> ExampleB<T> {
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
