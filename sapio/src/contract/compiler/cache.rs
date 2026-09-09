// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Caches for guards
use super::Context;
use crate::contract::actions::Guard;
use crate::contract::actions::SimpGen;
use crate::contract::CompilationError;
use sapio_base::effects::PathFragment;
use sapio_base::simp::GuardLT;
use sapio_base::simp::SIMPAttachableAt;
use sapio_base::Clause;
use std::collections::BTreeMap;
use std::sync::Arc;

pub type GuardSimps = Vec<Arc<dyn SIMPAttachableAt<GuardLT>>>;
pub(crate) enum CacheEntry<T> {
    Cached(Clause, Option<SimpGen<T>>),
    Fresh(fn(&T, Context) -> Clause, Option<SimpGen<T>>),
}

/// GuardCache assists with caching the computation of guard functions
/// during compilation.
pub(crate) struct GuardCache<T> {
    cache: BTreeMap<usize, Option<CacheEntry<T>>>,
}
impl<T> GuardCache<T> {
    pub fn new() -> Self {
        GuardCache {
            cache: BTreeMap::new(),
        }
    }
    pub(crate) fn get(
        &mut self,
        t: &T,
        f: fn() -> Option<Guard<T>>,
        ctx: Context,
        simp_ctx: Context,
    ) -> Result<Option<(Clause, GuardSimps)>, CompilationError> {
        let entry = self.cache.entry(f as usize).or_insert_with(|| match f()? {
            Guard::Cache(policy, simps) => Some(CacheEntry::Cached(policy(t), simps)),
            Guard::Fresh(policy, simps) => Some(CacheEntry::Fresh(policy, simps)),
        });
        let Some(entry) = entry else { return Ok(None) };
        let (clause, simps) = match entry {
            CacheEntry::Cached(clause, simps) => (clause.clone(), simps),
            CacheEntry::Fresh(policy, simps) => (policy(t, ctx), simps),
        };
        super::validation::validate_policy(&clause)?;
        let metadata = match simps {
            Some(generate) => generate(t, simp_ctx)?,
            None => vec![],
        };
        Ok(Some((clause, metadata)))
    }
}

pub(crate) fn create_guards<T>(
    self_ref: &T,
    mut ctx: Context,
    guards: &[fn() -> Option<Guard<T>>],
    gc: &mut GuardCache<T>,
) -> Result<(Clause, Vec<(Clause, GuardSimps)>), CompilationError> {
    let v = guards
        .iter()
        .enumerate()
        .map(|(i, guard)| {
            let mut guard_ctx = ctx.derive(PathFragment::Branch(i as u64))?;
            let simp_ctx = guard_ctx.derive(PathFragment::Metadata)?;
            gc.get(self_ref, *guard, guard_ctx, simp_ctx)
        })
        .filter_map(Result::transpose)
        .collect::<Result<Vec<_>, _>>()?;
    Ok((super::conjoin_guards(v.iter().map(|x| &x.0)), v))
}
