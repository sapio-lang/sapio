// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Conditions controlling whether an action is compiled and may return no templates.
use super::{CompilationError, Context};
use sapio_base::effects::PathFragment;

use std::collections::LinkedList;

const REQUIRED_NEVER_CONFLICT: &str = "Never and Required incompatible";
/// Conditional Compilation function has specified that compilation of this
/// function should be required or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConditionalCompileType {
    /// May proceed without calling this function at all
    Skippable,
    /// If no errors are returned, and no txtmpls are returned,
    /// it is not an error and the branch is pruned.
    Nullable,
    /// The default condition if no ConditionallyCompileIf function is set, the
    /// branch is present and it is required.
    Required,
    /// This branch must never be used
    Never,
    /// No Constraint, nothing is changed by this rule
    NoConstraint,
    /// The branch should always trigger an error, with some reasons
    Fail(LinkedList<String>),
}

impl ConditionalCompileType {
    /// Merge two conditional compilation decisions.
    ///
    /// Decision kinds are associative and commutative. Diagnostics preserve
    /// operand order, so those properties do not apply to the message lists:
    /// an existing `Fail` absorbs a later non-failing operand and its constraints.
    /// Compilation assembles the full declared condition list separately so
    /// an explicit failure cannot hide a `Required`/`Never` contradiction.
    ///
    /// Precedence:
    ///     Fail > non-Fail ==> Fail
    ///     forall X. X > NoConstraint ==> X
    ///     Required > {Skippable, Nullable} ==> Required
    ///     Skippable > Nullable ==> Skippable
    ///     Never >< Required ==> Fail
    ///     Never > {Skippable, Nullable}  ==> Never
    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (ConditionalCompileType::NoConstraint, x) => x,
            (x, ConditionalCompileType::NoConstraint) => x,
            // Merge error messages
            (ConditionalCompileType::Fail(mut v), ConditionalCompileType::Fail(mut v2)) => {
                ConditionalCompileType::Fail({
                    v.append(&mut v2);
                    v
                })
            }
            // Explicit failure overrides other conditions.
            (ConditionalCompileType::Fail(v), _) | (_, ConditionalCompileType::Fail(v)) => {
                ConditionalCompileType::Fail(v)
            }
            // Never and Required Conflict
            (ConditionalCompileType::Required, ConditionalCompileType::Never)
            | (ConditionalCompileType::Never, ConditionalCompileType::Required) => {
                let mut l = LinkedList::new();
                l.push_front(String::from(REQUIRED_NEVER_CONFLICT));
                ConditionalCompileType::Fail(l)
            }
            // Never stays Never
            (ConditionalCompileType::Never, ConditionalCompileType::Skippable)
            | (ConditionalCompileType::Skippable, ConditionalCompileType::Never)
            | (ConditionalCompileType::Never, ConditionalCompileType::Nullable)
            | (ConditionalCompileType::Nullable, ConditionalCompileType::Never)
            | (ConditionalCompileType::Never, ConditionalCompileType::Never) => {
                ConditionalCompileType::Never
            }
            // Required stays Required
            (ConditionalCompileType::Required, ConditionalCompileType::Skippable)
            | (ConditionalCompileType::Skippable, ConditionalCompileType::Required)
            | (ConditionalCompileType::Required, ConditionalCompileType::Nullable)
            | (ConditionalCompileType::Nullable, ConditionalCompileType::Required)
            | (ConditionalCompileType::Required, ConditionalCompileType::Required) => {
                ConditionalCompileType::Required
            }
            (ConditionalCompileType::Skippable, ConditionalCompileType::Skippable)
            | (ConditionalCompileType::Skippable, ConditionalCompileType::Nullable)
            | (ConditionalCompileType::Nullable, ConditionalCompileType::Skippable) => {
                ConditionalCompileType::Skippable
            }
            (ConditionalCompileType::Nullable, ConditionalCompileType::Nullable) => {
                ConditionalCompileType::Nullable
            }
        }
    }
}

/// A `ConditionallyCompileIf` is a function wrapper which generates some
/// condition that must be met to disable a branch.
///
/// We use a separate function so that static analysis tools may operate without
/// running the transaction-generation callback.
pub enum ConditionallyCompileIf<ContractSelf> {
    /// Fresh Variant may be called repeatedly
    Fresh(fn(&ContractSelf, Context) -> ConditionalCompileType),
}

/// A List of ConditionallyCompileIfs, for convenience
pub type ConditionallyCompileIfList<'a, T> = &'a [fn() -> Option<ConditionallyCompileIf<T>>];

pub(crate) struct CCILWrapper<'a, T>(pub ConditionallyCompileIfList<'a, T>);

impl<'a, T> CCILWrapper<'a, T> {
    /// Evaluate present conditions at their declared slots. Explicit errors
    /// retain declaration order; a contradictory requirement is reported once,
    /// after those errors, regardless of where an explicit failure appeared.
    pub fn assemble(
        &self,
        self_ref: &T,
        context: &mut Context,
    ) -> Result<ConditionalCompileType, CompilationError> {
        let mut decision = ConditionalCompileType::NoConstraint;
        let mut required = false;
        let mut never = false;
        // Some(empty) still represents an explicit Fail with no diagnostics.
        let mut errors: Option<LinkedList<String>> = None;
        for (slot, factory) in self.0.iter().enumerate() {
            let Some(ConditionallyCompileIf::Fresh(condition)) = factory() else {
                continue;
            };
            let child = context.derive(PathFragment::Branch(slot as u64))?;
            match condition(self_ref, child) {
                ConditionalCompileType::Required => required = true,
                ConditionalCompileType::Never => never = true,
                ConditionalCompileType::Fail(mut messages) => {
                    errors
                        .get_or_insert_with(LinkedList::new)
                        .append(&mut messages);
                }
                constraint => decision = decision.merge(constraint),
            }
        }
        if required && never {
            errors
                .get_or_insert_with(LinkedList::new)
                .push_back(REQUIRED_NEVER_CONFLICT.into());
        }
        Ok(match errors {
            Some(messages) => ConditionalCompileType::Fail(messages),
            None if required => ConditionalCompileType::Required,
            None if never => ConditionalCompileType::Never,
            None => decision,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{Amount, Network};
    use sapio_base::LoweringPlan;
    use std::cell::Cell;
    use std::sync::Arc;

    fn context() -> Context {
        Context::new(
            Network::Regtest,
            Amount::ZERO,
            LoweringPlan::Native,
            "conditions".try_into().unwrap(),
            Arc::new(Default::default()),
            None,
        )
    }

    fn absent() -> Option<ConditionallyCompileIf<Cell<usize>>> {
        None
    }

    fn counted() -> Option<ConditionallyCompileIf<Cell<usize>>> {
        Some(ConditionallyCompileIf::Fresh(|calls, _| {
            calls.set(calls.get() + 1);
            ConditionalCompileType::NoConstraint
        }))
    }

    fn failing() -> Option<ConditionallyCompileIf<Cell<usize>>> {
        Some(ConditionallyCompileIf::Fresh(|calls, _| {
            calls.set(calls.get() + 1);
            ConditionalCompileType::Fail(LinkedList::new())
        }))
    }

    #[test]
    fn occupied_condition_slot_returns_its_derivation_error_without_skipping() {
        let mut context = context();
        context.derive_num(1u64).unwrap();
        let calls = Cell::new(0);
        let result = CCILWrapper(&[absent, counted, counted]).assemble(&calls, &mut context);
        assert!(matches!(
            result,
            Err(CompilationError::ContexPathAlreadyDerived)
        ));
        assert_eq!(calls.get(), 0);
        assert!(context.derive_num(0u64).is_ok());
        assert!(context.derive_num(2u64).is_ok());
    }

    #[test]
    fn explicit_failure_does_not_hide_a_later_derivation_error() {
        let mut context = context();
        context.derive_num(1u64).unwrap();
        let calls = Cell::new(0);
        let result = CCILWrapper(&[failing, counted]).assemble(&calls, &mut context);
        assert!(matches!(
            result,
            Err(CompilationError::ContexPathAlreadyDerived)
        ));
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn absent_factories_do_not_reserve_contexts() {
        let mut context = context();
        let calls = Cell::new(0);
        assert_eq!(
            CCILWrapper(&[absent, absent])
                .assemble(&calls, &mut context)
                .unwrap(),
            ConditionalCompileType::NoConstraint
        );
        assert_eq!(calls.get(), 0);
        assert!(context.derive_num(0u64).is_ok());
        assert!(context.derive_num(1u64).is_ok());
    }
}
