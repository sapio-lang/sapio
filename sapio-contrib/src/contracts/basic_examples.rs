// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Some basic examples showing a kitchen sink of functionality
use super::*;
use sapio::contract::actions::ConditionalCompileType;
use sapio_base::timelocks::RelTime;
use std::collections::LinkedList;
use std::convert::TryFrom;
use std::marker::PhantomData;

#[derive(JsonSchema, Serialize, Deserialize)]
struct ExampleA {
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    amount: CoinAmount,
    resolution: Compiled,
}

impl ExampleA {
    guard! {fn timeout(self, _ctx) { Clause::Older(100) }}
    guard! {cached fn signed(self, _ctx) {Clause::And(vec![Clause::Key(self.alice), Clause::Key(self.bob)])}}
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
    guard! {cached fn all_signed(self, _ctx) {Clause::Threshold(T::get_n(self.threshold, self.participants.len()as u8) as usize, self.participants.iter().map(|k| Clause::Key(*k)).collect())}}
}

impl ExampleBThen for ExampleB<Finish> {}
impl ExampleBThen for ExampleB<Start> {
    then! {fn begin_contest(self, ctx) {
        ctx.template().add_output(
            self.amount.try_into()?,
            &ExampleB::<Finish> {
                participants: self.participants.clone(),
                threshold: self.threshold,
                amount: self.amount,
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

/// Trustless Escrowing Contract
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct ExampleCompileIf {
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    alice_escrow: (CoinAmount, bitcoin::Address),
    bob_escrow: (CoinAmount, bitcoin::Address),
    escrow_disable: bool,
    escrow_required_no_conflict_disabled: bool,
    escrow_required_conflict_disabled: bool,
    escrow_nullable: bool,
    escrow_error: Option<String>,
}

impl ExampleCompileIf {
    guard! { fn cooperate(self, _ctx) { Clause::And(vec![Clause::Key(self.alice), Clause::Key(self.bob)]) } }
    compile_if!(
        /// `should_escrow` disables any branch depending on it. If not set,
        /// it checks to make the branch required. This is done in a conflict-free way;
        /// that is that  if escrow_required_no_conflict_disabled is set and escrow_disable
        /// is set there is no problem.
        fn should_escrow(self, _ctx) {
            if self.escrow_disable {
                ConditionalCompileType::Never
            } else {
                if self.escrow_required_no_conflict_disabled {
                    ConditionalCompileType::Required
                } else {
                    ConditionalCompileType::Skippable
                }
            }
        }
    );
    compile_if! {
        /// `must_escrow` requires that any depending branch be taken.
        /// It may conflict with escrow_disable, if they are both set then
        /// compilation will fail.
        fn must_escrow(self, _ctx) {
            if self.escrow_required_conflict_disabled {
                ConditionalCompileType::Required
            } else {
                ConditionalCompileType::NoConstraint
            }
        }
    }
    compile_if! {
        /// `escrow_nullable_ok` tells the compiler if it is OK if dependents on this
        /// condition return 0 txiter items -- if so, the entire branch is pruned.
        fn escrow_nullable_ok(self, _ctx) {
            if self.escrow_nullable {
                ConditionalCompileType::Nullable
            } else {
                ConditionalCompileType::NoConstraint
            }
        }
    }

    compile_if! {
        /// `escrow_error_chk` fails with the provided error, if any
        fn escrow_error_chk(self, _ctx) {
            if let Some(e) = &self.escrow_error {
                let mut l = LinkedList::new();
                l.push_front(e.clone());
                ConditionalCompileType::Fail(l)
            } else {
                ConditionalCompileType::NoConstraint
            }
        }
    }
    then! {compile_if: [Self::should_escrow, Self::must_escrow, Self::escrow_nullable_ok, Self::escrow_error_chk]
        fn use_escrow(self, ctx) {
        ctx.template()
            .add_output(
                self.alice_escrow.0.try_into()?,
                &Compiled::from_address(self.alice_escrow.1.clone(), None),
                None)?
            .add_output(
                self.bob_escrow.0.try_into()?,
                &Compiled::from_address(self.bob_escrow.1.clone(), None),
                None)?
            .set_sequence(0, RelTime::try_from(std::time::Duration::from_secs(10*24*60*60))?.into())?.into()
    }}
}

impl Contract for ExampleCompileIf {
    declare! {finish, Self::cooperate}
    declare! {then, Self::use_escrow}
    declare! {non updatable}
}
