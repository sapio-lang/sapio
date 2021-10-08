// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A decorator which can be used to skip clausesd based on a computation.
use super::Context;
use sapio_base::effects::PathFragment;

use std::collections::LinkedList;
/// Conditional Compilation function has specified that compilation of this
/// function should be required or not.
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
    /// Merge two `ConditionalCompileTypes` into one conditions.
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
            // Fail ignored and overrides other conditions.
            (ConditionalCompileType::Fail(v), _) | (_, ConditionalCompileType::Fail(v)) => {
                ConditionalCompileType::Fail(v)
            }
            // Never and Required Conflict
            (ConditionalCompileType::Required, ConditionalCompileType::Never)
            | (ConditionalCompileType::Never, ConditionalCompileType::Required) => {
                let mut l = LinkedList::new();
                l.push_front(String::from("Never and Required incompatible"));
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
/// running the actual `ThenFunc`.
pub enum ConditionallyCompileIf<ContractSelf> {
    /// Fresh Variant may be called repeatedly
    Fresh(fn(&ContractSelf, Context) -> ConditionalCompileType),
}

/// A List of ConditionallyCompileIfs, for convenience
pub type ConditionallyCompileIfList<'a, T> = &'a [fn() -> Option<ConditionallyCompileIf<T>>];

pub(crate) struct CCILWrapper<'a, T>(pub ConditionallyCompileIfList<'a, T>);

impl<'a, T> CCILWrapper<'a, T> {
    /// Assembles the list by folding merge over it
    pub fn assemble(&self, self_ref: &T, context: &mut Context) -> ConditionalCompileType {
        self.0
            .iter()
            .filter_map(|compf| compf())
            .zip((0..).flat_map(|i| context.derive(PathFragment::Branch(i)).ok()))
            .fold(ConditionalCompileType::NoConstraint, |acc, (cond, c)| {
                let ConditionallyCompileIf::Fresh(f) = cond;
                acc.merge(f(self_ref, c))
            })
    }
}
