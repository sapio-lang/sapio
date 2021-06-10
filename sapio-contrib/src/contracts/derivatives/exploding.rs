// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Contracts which have a expiration date before which they must be executed...
use super::*;
use sapio_base::timelocks::*;
/// Generic functionality required for Exploding contracts
pub trait Explodes: 'static + Sized {
    then! {
        /// What to do when the timeout expires
        explodes
    }
    then! {
        /// what to do when the holder wishes to strike
        strikes
    }
}

impl<T> Contract for ExplodingOption<T>
where
    GenericBet: TryFrom<T, Error = CompilationError>,
    T: Clone + 'static,
{
    declare!(then, Self::explodes, Self::strikes);
    declare!(non updatable);
}

impl<T> Contract for UnderFundedExplodingOption<T>
where
    GenericBet: TryFrom<T, Error = CompilationError>,
    T: Clone + 'static,
{
    declare!(then, Self::explodes, Self::strikes);
    declare!(non updatable);
}
/// Wraps a generic option opt with functionality to refund both parties on timeout.
pub struct ExplodingOption<T: 'static> {
    party_one: Amount,
    party_two: Amount,
    key_p1: bitcoin::Address,
    key_p2: bitcoin::Address,
    key_p2_pk: Clause,
    opt: T,
    timeout: AnyAbsTimeLock,
}

impl<T> ExplodingOption<T> {
    guard! {fn signed(self, _ctx) { self.key_p2_pk.clone() }}
}
impl<T> Explodes for ExplodingOption<T>
where
    GenericBet: TryFrom<T, Error = CompilationError>,
    T: Clone,
{
    then! {
        fn explodes (self, ctx) {
            ctx.template()
                .add_output(
                    self.party_one.into(),
                    &Compiled::from_address(self.key_p1.clone(), None),
                    None,
                )?
                .add_output(
                    self.party_two.into(),
                    &Compiled::from_address(self.key_p2.clone(), None),
                    None,
                )?
                .set_lock_time(self.timeout)?
                .into()
        }
    }

    then! {
        guarded_by: [Self::signed]
        fn strikes(self, ctx) {
            ctx.template()
                .add_output(
                    (self.party_one + self.party_two).into(),
                    &GenericBet::try_from(self.opt.clone())?,
                    None,
                )?
                .into()
        }
    }
}

/// Similar to `ExplodingOption` except that the option requires an additional
/// value amount to be paid in in order to execute, hence being "under funded"
pub struct UnderFundedExplodingOption<T: 'static> {
    party_one: Amount,
    party_two: Amount,
    key_p1: bitcoin::Address,
    opt: T,
    timeout: AnyAbsTimeLock,
}

impl<T> Explodes for UnderFundedExplodingOption<T>
where
    GenericBet: TryFrom<T, Error = CompilationError>,
    T: Clone,
{
    then! {
        fn explodes(self, ctx) {
            Ok(Box::new(std::iter::once(
                ctx.template()
                    .add_output(
                        self.party_one.into(),
                        &Compiled::from_address(self.key_p1.clone(), None),
                        None,
                    )?
                    .set_lock_time(self.timeout)?
                    .into(),
            )))
        }
    }

    then! {
        fn strikes(self, ctx) {
            ctx.template()
                .add_amount(self.party_two)
                .add_sequence()
                .add_output(
                    (self.party_one + self.party_two).into(),
                    &GenericBet::try_from(self.opt.clone())?,
                    None,
                )?
                .into()
        }
    }
}
