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
    then!(
        /// What to do when the timeout expires
        explodes
    );
    then!(
        /// what to do when the holder wishes to strike
        strikes
    );
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
    guard!(signed | s, ctx | { s.key_p2_pk.clone() });
}
impl<T> Explodes for ExplodingOption<T>
where
    GenericBet: TryFrom<T, Error = CompilationError>,
    T: Clone,
{
    then!(
        explodes | s,
        ctx | {
            ctx.template()
                .add_output(
                    s.party_one.into(),
                    &Compiled::from_address(s.key_p1.clone(), None),
                    None,
                )?
                .add_output(
                    s.party_two.into(),
                    &Compiled::from_address(s.key_p2.clone(), None),
                    None,
                )?
                .set_lock_time(s.timeout)?
                .into()
        }
    );

    then!(
        strikes[Self::signed] | s,
        ctx | {
            ctx.template()
                .add_output(
                    (s.party_one + s.party_two).into(),
                    &GenericBet::try_from(s.opt.clone())?,
                    None,
                )?
                .into()
        }
    );
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
    then!(
        explodes | s,
        ctx | {
            Ok(Box::new(std::iter::once(
                ctx.template()
                    .add_output(
                        s.party_one.into(),
                        &Compiled::from_address(s.key_p1.clone(), None),
                        None,
                    )?
                    .set_lock_time(s.timeout)?
                    .into(),
            )))
        }
    );

    then!(
        strikes | s,
        ctx | {
            ctx.template()
                .add_amount(s.party_two)
                .add_sequence()
                .add_output(
                    (s.party_one + s.party_two).into(),
                    &GenericBet::try_from(s.opt.clone())?,
                    None,
                )?
                .into()
        }
    );
}
