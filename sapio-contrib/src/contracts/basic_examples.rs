// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Some basic examples showing a kitchen sink of functionality
use super::*;
use sapio::contract::actions::ConditionalCompileType;
use sapio_base::miniscript::{Threshold, ThresholdError};
use sapio_base::timelocks::RelTime;
use sapio_macros::compile_if;
use sapio_macros::guard;
use std::collections::LinkedList;
use std::convert::TryFrom;
use std::marker::PhantomData;

#[derive(JsonSchema, Serialize, Deserialize)]
struct ExampleA {
    #[schemars(with = "String")]
    alice: bitcoin::XOnlyPublicKey,
    #[schemars(with = "String")]
    bob: bitcoin::XOnlyPublicKey,
}

impl ExampleA {
    #[guard]
    fn timeout(self, _ctx: sapio::Context) {
        Clause::And(vec![
            Clause::Key(self.bob).into(),
            Clause::try_from(sapio_base::timelocks::RelHeight::from(100))
                .expect("positive constant locktime")
                .into(),
        ])
    }
    #[guard(cached)]
    fn signed(self) {
        Clause::And(vec![
            Clause::Key(self.alice).into(),
            Clause::Key(self.bob).into(),
        ])
    }
}

impl Contract for ExampleA {
    declare! {finish, Self::signed, Self::timeout}
    declare! {non updatable}
}

trait BState: JsonSchema {
    fn get_n(_n: usize, max: usize) -> usize {
        max
    }
}
#[derive(JsonSchema, Serialize, Deserialize)]
struct Start;
impl BState for Start {}
#[derive(JsonSchema, Serialize, Deserialize)]
struct Finish;
impl BState for Finish {
    fn get_n(n: usize, _max: usize) -> usize {
        n
    }
}

trait ExampleBThen
where
    Self: Sized + Contract,
{
    decl_then! {begin_contest}
}

#[derive(JsonSchema, Serialize, Deserialize)]
struct ExampleB<T: BState> {
    #[schemars(with = "Vec<String>")]
    participants: Vec<bitcoin::XOnlyPublicKey>,
    threshold: u8,
    amount: CoinAmount,
    #[serde(skip)]
    pd: PhantomData<T>,
}

impl<T: BState> ExampleB<T> {
    #[guard(policy, cached)]
    fn all_signed(self) -> Result<Clause, ThresholdError> {
        Ok(Clause::Thresh(Threshold::new(
            T::get_n(self.threshold as usize, self.participants.len()),
            self.participants
                .iter()
                .map(|k| Clause::Key(*k))
                .map(Into::into)
                .collect(),
        )?))
    }
}

impl ExampleBThen for ExampleB<Finish> {}
impl ExampleBThen for ExampleB<Start> {
    #[then]
    fn begin_contest(self, ctx: sapio::Context) {
        ctx.template()
            .add_output(
                self.amount.try_into()?,
                &ExampleB::<Finish> {
                    participants: self.participants.clone(),
                    threshold: self.threshold,
                    amount: self.amount,
                    pd: Default::default(),
                },
                None,
            )?
            .into()
    }
}

impl<T: BState> Contract for ExampleB<T>
where
    ExampleB<T>: ExampleBThen + 'static,
{
    declare! {then, Self::begin_contest}
    declare! {finish, Self::all_signed}
    declare! {non updatable }

    fn ensure_amount(&self, _ctx: Context) -> Result<bitcoin::Amount, CompilationError> {
        if self.threshold == 0 || self.threshold as usize > self.participants.len() {
            return Err(CompilationError::Custom(
                "Threshold must select at least one participant".into(),
            ));
        }
        Ok(self.amount.try_into()?)
    }
}

/// Trustless Escrowing Contract
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct ExampleCompileIf {
    #[schemars(with = "String")]
    alice: bitcoin::XOnlyPublicKey,
    #[schemars(with = "String")]
    bob: bitcoin::XOnlyPublicKey,
    #[schemars(with = "(CoinAmount, String)")]
    alice_escrow: (
        CoinAmount,
        bitcoin::Address<bitcoin::address::NetworkUnchecked>,
    ),
    #[schemars(with = "(CoinAmount, String)")]
    bob_escrow: (
        CoinAmount,
        bitcoin::Address<bitcoin::address::NetworkUnchecked>,
    ),
    escrow_disable: bool,
    escrow_required_no_conflict_disabled: bool,
    escrow_required_conflict_disabled: bool,
    escrow_nullable: bool,
    escrow_error: Option<String>,
}

impl ExampleCompileIf {
    #[guard]
    fn cooperate(self, _ctx: Context) {
        Clause::And(vec![
            Clause::Key(self.alice).into(),
            Clause::Key(self.bob).into(),
        ])
    }
    /// `should_escrow` disables any branch depending on it. If not set,
    /// it checks to make the branch required. This is done in a conflict-free way;
    /// that is that  if escrow_required_no_conflict_disabled is set and escrow_disable
    /// is set there is no problem.
    #[compile_if]
    fn should_escrow(self, _ctx: Context) {
        if self.escrow_disable {
            ConditionalCompileType::Never
        } else if self.escrow_required_no_conflict_disabled {
            ConditionalCompileType::Required
        } else {
            ConditionalCompileType::Skippable
        }
    }
    /// `must_escrow` requires that any depending branch be taken.
    /// It may conflict with escrow_disable, if they are both set then
    /// compilation will fail.
    #[compile_if]
    fn must_escrow(self, _ctx: Context) {
        if self.escrow_required_conflict_disabled {
            ConditionalCompileType::Required
        } else {
            ConditionalCompileType::NoConstraint
        }
    }
    /// `escrow_nullable_ok` tells the compiler if it is OK if dependents on this
    /// condition return 0 txiter items -- if so, the entire branch is pruned.
    #[compile_if]
    fn escrow_nullable_ok(self, _ctx: Context) {
        if self.escrow_nullable {
            ConditionalCompileType::Nullable
        } else {
            ConditionalCompileType::NoConstraint
        }
    }

    /// `escrow_error_chk` fails with the provided error, if any
    #[compile_if]
    fn escrow_error_chk(self, _ctx: Context) {
        if let Some(e) = &self.escrow_error {
            let mut l = LinkedList::new();
            l.push_front(e.clone());
            ConditionalCompileType::Fail(l)
        } else {
            ConditionalCompileType::NoConstraint
        }
    }
    #[then(
        compile_if = "[Self::should_escrow, Self::must_escrow, Self::escrow_nullable_ok, Self::escrow_error_chk]"
    )]
    fn use_escrow(self, ctx: sapio::Context) {
        let network = ctx.network;
        ctx.template()
            .add_output(
                self.alice_escrow.0.try_into()?,
                &Compiled::from_address(
                    self.alice_escrow.1.clone().require_network(network)?,
                    bitcoin::Amount::ZERO,
                ),
                None,
            )?
            .add_output(
                self.bob_escrow.0.try_into()?,
                &Compiled::from_address(
                    self.bob_escrow.1.clone().require_network(network)?,
                    bitcoin::Amount::ZERO,
                ),
                None,
            )?
            .set_sequence(
                0,
                RelTime::try_from(std::time::Duration::from_secs(10 * 24 * 60 * 60))?.into(),
            )?
            .into()
    }
}

impl Contract for ExampleCompileIf {
    declare! {finish, Self::cooperate}
    declare! {then, Self::use_escrow}
    declare! {non updatable}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{address, context, key};

    fn escrow() -> ExampleCompileIf {
        ExampleCompileIf {
            alice: key(1),
            bob: key(2),
            alice_escrow: (
                bitcoin::Amount::from_sat(400).into(),
                address(1).into_unchecked(),
            ),
            bob_escrow: (
                bitcoin::Amount::from_sat(600).into(),
                address(2).into_unchecked(),
            ),
            escrow_disable: false,
            escrow_required_no_conflict_disabled: false,
            escrow_required_conflict_disabled: false,
            escrow_nullable: false,
            escrow_error: None,
        }
    }

    #[test]
    fn guards_and_type_state_contest_compile() {
        let a = ExampleA {
            alice: key(1),
            bob: key(2),
        };
        assert_eq!(
            a.guard_signed(),
            Clause::And(vec![Clause::Key(key(1)).into(), Clause::Key(key(2)).into()])
        );
        assert_eq!(
            a.guard_timeout(context(0)),
            Clause::And(vec![
                Clause::Key(key(2)).into(),
                Clause::try_from(sapio_base::timelocks::RelHeight::from(100))
                    .expect("positive constant locktime")
                    .into()
            ])
        );
        a.compile(context(1000)).unwrap().validate().unwrap();
        let b = ExampleB::<Start> {
            participants: vec![key(1), key(2)],
            threshold: 1,
            amount: bitcoin::Amount::from_sat(1000).into(),
            pd: PhantomData,
        };
        let object = b.compile(context(1000)).unwrap();
        object.validate().unwrap();
        assert_eq!(
            object.ctv_to_tx.values().next().unwrap().tx.output[0]
                .value
                .to_sat(),
            1000
        );
        assert_eq!(
            b.guard_all_signed().unwrap(),
            Clause::Thresh(
                Threshold::new(
                    2,
                    vec![Clause::Key(key(1)).into(), Clause::Key(key(2)).into()]
                )
                .expect("valid threshold")
            )
        );
    }

    #[test]
    fn thresholds_do_not_truncate_and_invalid_quorums_fail() {
        let mut b = ExampleB::<Start> {
            participants: vec![key(1); 256],
            threshold: 1,
            amount: bitcoin::Amount::from_sat(1000).into(),
            pd: PhantomData,
        };
        assert!(
            matches!(b.guard_all_signed().unwrap(), Clause::Thresh(ref quorum) if quorum.k() == 256)
        );
        b.threshold = 0;
        assert!(b.compile(context(1000)).is_err());
        b.threshold = 2;
        b.participants.truncate(1);
        assert!(b.compile(context(1000)).is_err());
        b.participants.clear();
        assert!(b.guard_all_signed().is_err());
    }

    #[test]
    fn conditional_branches_enforce_disabling_required_and_error_states() {
        let mut contract = escrow();
        assert!(contract
            .compile(context(1000))
            .unwrap()
            .ctv_to_tx
            .is_empty());
        contract.escrow_required_no_conflict_disabled = true;
        assert_eq!(contract.compile(context(1000)).unwrap().ctv_to_tx.len(), 1);
        contract.escrow_disable = true;
        assert!(contract
            .compile(context(1000))
            .unwrap()
            .ctv_to_tx
            .is_empty());
        contract.escrow_required_conflict_disabled = true;
        assert!(contract.compile(context(1000)).is_err());
        contract.escrow_disable = false;
        contract.escrow_error = Some("explicit rejection".into());
        assert!(contract.compile(context(1000)).is_err());
    }
}
