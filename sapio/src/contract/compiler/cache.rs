// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Caches for guards
use super::Context;
use super::InternalCompilerTag;
use crate::contract::actions::Guard;
use crate::contract::actions::SimpGen;
use crate::contract::CompilationError;
use sapio_base::effects::PathFragment;
use sapio_base::simp::GuardLT;
use sapio_base::simp::SIMPAttachableAt;
use sapio_base::Clause;
use std::collections::BTreeMap;

pub type GuardSimps = Vec<Box<dyn SIMPAttachableAt<GuardLT>>>;
pub(crate) enum CacheEntry<T> {
    Cached(Clause, GuardSimps),
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
    pub(crate) fn create_entry(
        g: Option<Guard<T>>,
        t: &T,
        ctx: Context,
        simp_ctx: Context,
    ) -> Result<Option<CacheEntry<T>>, CompilationError> {
        match g {
            Some(Guard::Cache(f, Some(simp_gen))) => {
                Ok(Some(CacheEntry::Cached(f(t, ctx), simp_gen(t, simp_ctx)?)))
            }
            Some(Guard::Cache(f, None)) => Ok(Some(CacheEntry::Cached(f(t, ctx), vec![]))),
            Some(Guard::Fresh(f, simp_gen)) => Ok(Some(CacheEntry::Fresh(f, simp_gen))),
            None => Ok(None),
        }
    }
    pub(crate) fn get(
        &mut self,
        t: &T,
        f: fn() -> Option<Guard<T>>,
        ctx: Context,
        simp_ctx: Context,
    ) -> Result<Option<(Clause, GuardSimps)>, CompilationError> {
        let mut entry = self.cache.entry(f as usize);
        let r = match entry {
            std::collections::btree_map::Entry::Vacant(v) => {
                let ent = Self::create_entry(
                    f(),
                    t,
                    ctx.internal_clone(InternalCompilerTag { _secret: () }),
                    simp_ctx.internal_clone(InternalCompilerTag { _secret: () }),
                )?;
                let r = v.insert(ent);
                r
            }
            std::collections::btree_map::Entry::Occupied(ref mut o) => o.get_mut(),
        };
        match r {
            Some(CacheEntry::Cached(s, v)) => Ok(Some((
                s.clone(),
                v.iter().map(|e| e.make_clone()).collect(),
            ))),
            Some(CacheEntry::Fresh(f, s)) => Ok(Some((
                f(t, ctx),
                match s {
                    Some(f2) => f2(t, simp_ctx)?,
                    None => vec![],
                },
            ))),
            None => Ok(None),
        }
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
        .zip((0..).flat_map(|i| {
            let new = ctx.derive(PathFragment::Branch(i)).ok()?;
            let simp = ctx.derive(PathFragment::Metadata).ok()?;
            Some((new, simp))
        }))
        .filter_map(|(x, (c, simp_c))| gc.get(self_ref, *x, c, simp_c).transpose())
        .collect::<Result<Vec<_>, _>>()?;
    let mut clauses: Vec<_> = v
        .iter()
        .map(|x| &x.0)
        .filter(|x| **x != Clause::Trivial)
        .map(|x| x.clone())
        .collect(); // no point in using any Trivials
    if clauses.len() == 0 {
        Ok((Clause::Trivial, v))
    } else if clauses.len() == 1 {
        Ok((clauses.pop().unwrap(), v))
    } else {
        Ok((Clause::And(clauses), v))
    }
}
