// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! An example of how one might begin building a payment channel contract in Sapio using
//! Eltoo

use contract::*;
use sapio::contract::actions::ConditionalCompileType;
use sapio::contract::error::CompilationError;
use sapio::template::Output;
use sapio::*;
use sapio_base::timelocks::RelHeight;
use sapio_base::Clause;
use sapio_macros::compile_if;
use sapio::contract::Context;

use bitcoin;
use sapio_base::timelocks::{AbsTime, AnyAbsTimeLock, BIG_PAST_DATE, START_OF_TIME};

use std::convert::TryFrom;
use std::convert::TryInto;

/// Args are some messages that can be passed to a Channel instance
#[derive(Clone)]
pub struct Update {
    /// the balances of the channel
    resolution: Vec<Output>,
    /// the channel seq, guaranteed to be > 500_000_000
    sequence: AbsTime,
    /// the amount of timeout before this update can be claimed
    maturity: RelHeight,
}

#[derive(Clone)]
struct OpenChannel {
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    alice_u: bitcoin::PublicKey,
    bob_u: bitcoin::PublicKey,
    pending_update: Option<Update>,
    min_maturity: RelHeight,
}
impl OpenChannel {
    #[guard]
    fn signed_update(self, _ctx: Context) {
        Clause::And(vec![Clause::Key(self.alice_u), Clause::Key(self.bob_u)])
    }
    #[guard]
    fn newer_sequence_check(self, _ctx: Context) {
        if let Some(prior) = self.pending_update.as_ref() {
            AbsTime::try_from(prior.sequence.get() + 1)
                .map(Clause::from)
                .unwrap_or(Clause::Unsatisfiable)
        } else {
            START_OF_TIME.into()
        }
    }
    finish! {
        guarded_by: [Self::signed_update, Self::newer_sequence_check]
        coerce_args: default_coerce
        fn update_state(self, ctx, o: Option<Update>) {
            if let Some(update) = o {
                if update.sequence > BIG_PAST_DATE  {
                    Err(CompilationError::TerminateCompilation)?;
                }
                let prior_seq =
                    self.pending_update.as_ref().map(|u|u.sequence).unwrap_or(1u32.try_into()?);
                if update.sequence <= prior_seq {
                    Err(CompilationError::TerminateCompilation)?;
                }

                let f = ctx.funds();
                ctx.template()
                .set_lock_time(AnyAbsTimeLock::from(update.sequence.clone()))?
                .add_output(
                    f,
                    &OpenChannel {
                        pending_update: Some(update.clone()), ..self.clone()
                    },
                    None
                )?.into()
            } else {
                Ok(Box::new(std::iter::empty()))
            }
        }
    }

    #[compile_if]
    fn triggered(self, _ctx: Context) {
        if self.pending_update.is_some() {
            ConditionalCompileType::NoConstraint
        } else {
            ConditionalCompileType::Never
        }
    }
    fn get_maturity(&self) -> RelHeight {
        self.pending_update
            .as_ref()
            .map(|u| std::cmp::max(u.maturity, self.min_maturity))
            .unwrap_or(self.min_maturity)
    }
    #[guard]
    fn timeout(self, _ctx: Context) {
        self.get_maturity().into()
    }
    then! {
        compile_if: [Self::triggered]
        guarded_by: [Self::timeout]
        fn complete_update(self, ctx) {
            let mut template = ctx.template().set_sequence(-1, self.get_maturity().into())?;
            for out in self.pending_update.as_ref().map(|p| &p.resolution).unwrap_or(&vec![]).iter() {
                template = template.add_output(out.amount, &out.contract, Some(out.metadata.clone()))?;
            }
            template.into()
        }
    }

    #[guard]
    fn sign_cooperative_close(self, _ctx: Context) {
        Clause::And(vec![Clause::Key(self.alice), Clause::Key(self.bob)])
    }

    #[compile_if]
    fn untriggered(self, _ctx: Context) {
        if self.pending_update.is_some() {
            ConditionalCompileType::Never
        } else {
            ConditionalCompileType::NoConstraint
        }
    }
    finish! {
        compile_if: [Self::untriggered]
        guarded_by: [Self::sign_cooperative_close]
        coerce_args: default_coerce
        fn coop_close(self, _ctx, _o: Option<Update>) {
            Ok(Box::new(std::iter::empty()))
        }
    }
}
/// Helper
fn default_coerce(
    k: <OpenChannel as Contract>::StatefulArguments,
) -> Result<Option<Update>, CompilationError> {
    Ok(k)
}

impl Contract for OpenChannel {
    declare! {updatable<Option<Update>>, Self::update_state,  Self::coop_close}
    declare! {then, Self::complete_update}
}
