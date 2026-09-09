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

use bitcoin;
use sapio_base::timelocks::{AbsTime, AnyAbsTimeLock, BIG_PAST_DATE, START_OF_TIME};

use std::convert::TryFrom;

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
    alice: bitcoin::XOnlyPublicKey,
    bob: bitcoin::XOnlyPublicKey,
    alice_u: bitcoin::XOnlyPublicKey,
    bob_u: bitcoin::XOnlyPublicKey,
    pending_update: Option<Update>,
    min_maturity: RelHeight,
}
impl OpenChannel {
    fn check_resolution(update: &Update, funds: bitcoin::Amount) -> Result<(), CompilationError> {
        let total = update
            .resolution
            .iter()
            .try_fold(bitcoin::Amount::ZERO, |total, output| {
                total
                    .checked_add(output.amount)
                    .ok_or(CompilationError::OutOfFunds)
            })?;
        if update.resolution.is_empty() || total != funds {
            return Err(CompilationError::Custom(
                "Channel resolution must preserve its balance".into(),
            ));
        }
        Ok(())
    }
    #[guard]
    fn signed_update(self, _ctx: Context) {
        Clause::And(vec![Clause::Key(self.alice_u), Clause::Key(self.bob_u)])
    }
    #[guard]
    fn newer_sequence_check(self, _ctx: Context) {
        if let Some(prior) = self.pending_update.as_ref() {
            prior
                .sequence
                .get()
                .checked_add(1)
                .and_then(|next| AbsTime::try_from(next).ok())
                .map(Clause::from)
                .unwrap_or(Clause::Unsatisfiable)
        } else {
            START_OF_TIME.into()
        }
    }
    #[continuation(
        guarded_by = "[Self::signed_update, Self::newer_sequence_check]",
        coerce_args = "default_coerce"
    )]
    fn update_state(self, ctx: sapio::Context, o: Option<Update>) {
        if let Some(update) = o {
            Self::check_resolution(&update, ctx.funds())?;
            if update.sequence > BIG_PAST_DATE {
                Err(CompilationError::TerminateCompilation)?;
            }
            let prior_seq = self
                .pending_update
                .as_ref()
                .map(|u| u.sequence)
                .unwrap_or(START_OF_TIME);
            if update.sequence <= prior_seq {
                Err(CompilationError::TerminateCompilation)?;
            }

            let f = ctx.funds();
            ctx.template()
                .set_lock_time(AnyAbsTimeLock::from(update.sequence))?
                .add_output(
                    f,
                    &OpenChannel {
                        pending_update: Some(update),
                        ..self.clone()
                    },
                    None,
                )?
                .into()
        } else {
            Ok(Box::new(std::iter::empty()))
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
    #[then(compile_if = "[Self::triggered]", guarded_by = "[Self::timeout]")]
    fn complete_update(self, ctx: sapio::Context) {
        let mut template = ctx
            .template()
            .set_sequence(-1, self.get_maturity().into())?;
        for out in self
            .pending_update
            .as_ref()
            .map(|p| &p.resolution)
            .unwrap_or(&vec![])
            .iter()
        {
            template =
                template.add_output(out.amount, &out.contract, Some(out.added_metadata.clone()))?;
        }
        template.into()
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
    #[continuation(
        compile_if = "[Self::untriggered]",
        guarded_by = "[Self::sign_cooperative_close]",
        coerce_args = "default_coerce"
    )]
    fn coop_close(self, ctx: sapio::Context, update: Option<Update>) {
        let Some(update) = update else { return empty() };
        Self::check_resolution(&update, ctx.funds())?;
        let mut template = ctx.template();
        for output in update.resolution {
            template = template.add_output(
                output.amount,
                &output.contract,
                Some(output.added_metadata),
            )?;
        }
        template.into()
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

    fn ensure_amount(&self, ctx: Context) -> Result<bitcoin::Amount, CompilationError> {
        if let Some(update) = &self.pending_update {
            Self::check_resolution(update, ctx.funds())?;
            if update.sequence > BIG_PAST_DATE {
                return Err(CompilationError::TerminateCompilation);
            }
        }
        Ok(ctx.funds())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{context, key};

    fn channel() -> OpenChannel {
        OpenChannel {
            alice: key(1),
            bob: key(2),
            alice_u: key(3),
            bob_u: key(4),
            pending_update: None,
            min_maturity: RelHeight::from(10),
        }
    }

    fn update(sequence: u32, funds: u64) -> Update {
        Update {
            sequence: AbsTime::try_from(sequence).unwrap(),
            maturity: RelHeight::from(5),
            resolution: vec![Output {
                amount: bitcoin::Amount::from_sat(funds),
                contract: key(1).compile(context(funds)).unwrap(),
                added_metadata: Default::default(),
            }],
        }
    }

    #[test]
    fn first_update_and_mature_resolution_preserve_funds() {
        let contract = channel();
        contract.compile(context(1000)).unwrap().validate().unwrap();
        let template = contract
            .continue_update_state(context(1000), Some(update(START_OF_TIME.get() + 1, 1000)))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(template.tx.lock_time, START_OF_TIME.get() + 1);
        let child = &template.outputs[0].contract;
        child.validate().unwrap();
        let payout = child.ctv_to_tx.values().next().unwrap();
        assert_eq!(payout.tx.input[0].sequence, 10);
        assert_eq!(payout.tx.output[0].value, 1000);
        assert_eq!(
            contract.guard_signed_update(context(0)),
            Clause::And(vec![Clause::Key(key(3)), Clause::Key(key(4))])
        );
    }

    #[test]
    fn stale_unbalanced_and_exhausted_updates_are_rejected() {
        let mut contract = channel();
        contract.pending_update = Some(update(START_OF_TIME.get() + 2, 1000));
        assert!(contract
            .continue_update_state(context(1000), Some(update(START_OF_TIME.get() + 2, 1000)))
            .is_err());
        assert!(contract
            .continue_update_state(context(1000), Some(update(START_OF_TIME.get() + 3, 999)))
            .is_err());
        contract.pending_update.as_mut().unwrap().sequence = AbsTime::try_from(u32::MAX).unwrap();
        assert_eq!(
            contract.guard_newer_sequence_check(context(0)),
            Clause::Unsatisfiable
        );
        let close = channel()
            .continue_coop_close(context(1000), Some(update(START_OF_TIME.get() + 1, 1000)))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(close.tx.output[0].value, 1000);
    }
}
