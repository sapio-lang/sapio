// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! a decorator type which is used to generate a spending condition
use std::sync::Arc;

use crate::contract::CompilationError;

use super::Context;

use sapio_base::{
    simp::{GuardLT, SIMPAttachableAt},
    Clause,
};
/// A spending condition, evaluated freshly or cached during one compilation.
///
/// Cached policies depend on contract data alone. Contextual metadata is
/// evaluated at every attachment, independently of whether the policy is cached.
pub enum Guard<ContractSelf> {
    /// Evaluate the policy once per guard declaration during this compilation.
    /// The policy cannot observe an attachment's path, effects or available funds.
    Cache(fn(&ContractSelf) -> Clause, Option<SimpGen<ContractSelf>>),
    /// Evaluate the policy with the context of each attachment.
    Fresh(
        fn(&ContractSelf, Context) -> Clause,
        Option<SimpGen<ContractSelf>>,
    ),
}

/// Generate guard metadata at its actual attachment context on every use.
pub type SimpGen<ContractSelf> =
    fn(
        cself: &ContractSelf,
        ctx: Context,
    ) -> Result<Vec<Arc<dyn SIMPAttachableAt<GuardLT>>>, CompilationError>;

/// A List of Guards, for convenience
pub type GuardList<'a, T> = &'a [fn() -> Option<Guard<T>>];
