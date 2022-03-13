// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A Contract for PowSwap Hashrate Derivatives
use bitcoin::util::amount::CoinAmount;
use bitcoin::Address;
use bitcoin::PublicKey;
use sapio::contract::actions::ConditionalCompileType;
use sapio::contract::*;
use sapio::template::Builder;
use sapio::template::Template;
use sapio::util::amountrange::AmountU64;
use sapio::*;
use sapio_base::timelocks::LockTimeError;
use sapio_base::timelocks::{
    AbsHeight, AbsTime, AnyAbsTimeLock, AnyRelTimeLock, AnyTimeLock, RelHeight, RelTime,
};
use sapio_base::Clause;
use schemars::*;
use serde::*;
use std::convert::{TryFrom, TryInto};
use std::time::Duration;

/// `ContractVariant` ensures that we either set a Relative Height and Absolute
/// Time or a Relative Time and Absolute Height, the two valid combinations, or
/// just one.
///
/// Note these are unlocking conditions for each participant.
///
/// Validity is ensured through smart constructor
#[derive(JsonSchema, Deserialize, Clone, Copy)]
#[serde(try_from = "ValidContractVariant")]
pub struct ContractVariant(Option<AnyRelTimeLock>, Option<AnyAbsTimeLock>);

/// In order to test for coherence here, we should convert
/// ValidContractVariant to ContractVariant.
///
/// The coherence rules should match one ruleset of:
/// - a single type of TimeLock (Relative Height, Relative Time, Absolute Time,
///   Absolute Height)
/// - a mixed TimeLock of just Relative Height/Absolute Time or just Relative
///   Time/Absolute Height
#[derive(JsonSchema, Deserialize, Clone)]
struct ValidContractVariant(Vec<AnyTimeLock>);

impl TryFrom<ValidContractVariant> for ContractVariant {
    type Error = CompilationError;
    fn try_from(vcv: ValidContractVariant) -> Result<Self, Self::Error> {
        let abs: Vec<_> = vcv
            .0
            .iter()
            .filter_map(|v| {
                if let AnyTimeLock::A(a) = v {
                    Some(a)
                } else {
                    None
                }
            })
            .collect();
        let rel: Vec<_> = vcv
            .0
            .iter()
            .filter_map(|v| {
                if let AnyTimeLock::R(r) = v {
                    Some(r)
                } else {
                    None
                }
            })
            .collect();

        let all_rh = rel.iter().all(|v| matches!(v, AnyRelTimeLock::RH(c)));
        let all_rt = rel.iter().all(|v| matches!(v, AnyRelTimeLock::RT(c)));
        #[derive(Debug)]
        struct LocalError(&'static str);
        impl std::fmt::Display for LocalError {
            fn fmt(
                &self,
                f: &mut std::fmt::Formatter<'_>,
            ) -> std::result::Result<(), std::fmt::Error> {
                self.0.fmt(f)
            }
        }
        impl std::error::Error for LocalError {}
        if !(all_rh || all_rt) {
            return Err(CompilationError::custom(LocalError(
                "Must have some timelock set!",
            )));
        }
        let all_ah = abs.iter().all(|v| matches!(v, AnyAbsTimeLock::AH(c)));
        let all_at = abs.iter().all(|v| matches!(v, AnyAbsTimeLock::AT(c)));
        if !(all_ah || all_at) {
            return Err(CompilationError::custom(LocalError(
                "Incoherent Absolute Timelocks (mixed height/time)",
            )));
        }

        let relative = rel.iter().max_by_key(|v| AnyRelTimeLock::get(v)).cloned();
        let absolute = abs.iter().max_by_key(|v| AnyAbsTimeLock::get(v)).cloned();

        if matches!((relative, absolute), (None, None)) {
            return Err(CompilationError::custom(LocalError(
                "Must have some timelock set!",
            )));
        }

        if (all_rt && all_at) || (all_rh && all_rt) {
            return Err(CompilationError::custom(LocalError(
                "Must mix {Relative,Absolute} Height and Absolute time!",
            )));
        }
        Ok(ContractVariant(relative.cloned(), absolute.cloned()))
    }
}

impl ContractVariant {
    fn get_relative(&self) -> AnyRelTimeLock {
        self.0.unwrap_or(RelTime::from(0).into())
    }
    fn get_abs(&self) -> AnyAbsTimeLock {
        self.1.unwrap_or(AbsHeight::try_from(0).unwrap().into())
    }
}

/// Instructions for a Payment from an outcome
#[derive(JsonSchema, Deserialize, Clone)]
pub struct Pays {
    sats: AmountU64,
    to: PublicKey,
}
/// A `Outcome` is a contract where
#[derive(JsonSchema, Deserialize, Clone)]
pub struct Outcome {
    /// # Variant
    /// if the base is time or height for the relative leg.
    unlocks_if: ContractVariant,
    /// # Outcome
    /// Payments to make (should be >= 1)
    outcome: Vec<Pays>,
}
/// A `PowSwap` is a contract where
#[derive(JsonSchema, Deserialize, Clone)]
pub struct PowSwap {
    /// # Parties
    pub outcomes: [Outcome; 2],
    /// # Cooperate Key
    coop: Vec<PublicKey>,
}

impl PowSwap {
    fn make_payoffs(&self, ctx: Context, payments: &[Pays]) -> Result<Builder, CompilationError> {
        let mut bld = ctx.template();
        for Pays { sats, to } in payments {
            bld = bld.add_output(sats.clone().into(), to, None)?;
        }
        Ok(bld)
    }
    #[then]
    fn payoff(self, mut base_ctx: Context) {
        let mut ret: Vec<Result<Template, _>> = vec![];
        for (i, path) in self.outcomes.iter().enumerate() {
            let ctx = base_ctx.derive_num(i as u64)?;
            let v = self
                .make_payoffs(ctx, &path.outcome)?
                .set_sequence(-1, path.unlocks_if.get_relative())?
                .set_lock_time(path.unlocks_if.get_abs())?
                .into();
            ret.push(Ok(v));
        }
        Ok(Box::new(ret.into_iter()))
    }
    #[guard]
    fn cooperate(self, ctx: Context) {
        Clause::And(self.coop.iter().cloned().map(Clause::Key).collect())
    }
}

impl Contract for PowSwap {
    declare! {then, Self::payoff}
    declare! {finish, Self::cooperate}
    declare! {non updatable}
}
