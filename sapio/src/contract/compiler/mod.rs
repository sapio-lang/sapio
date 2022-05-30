// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The primary compilation traits and types
use super::actions::ConditionalCompileType;
use super::AnyContract;
use super::CompilationError;
use super::Compiled;
use super::Context;
use crate::contract::abi::continuation::ContinuationPoint;
use crate::contract::actions::conditional_compile::CCILWrapper;
use crate::contract::actions::CallableAsFoF;

use crate::contract::TxTmplIt;
use crate::util::amountrange::AmountRange;
use ::miniscript::descriptor::TapTree;
use ::miniscript::*;
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::Hash;
use bitcoin::schnorr::TweakedPublicKey;
use sapio_base::reverse_path::ReversePath;
use std::collections::BinaryHeap;

use bitcoin::XOnlyPublicKey;
use sapio_base::effects::EffectDB;
use sapio_base::effects::EffectPath;
use sapio_base::effects::PathFragment;
use sapio_base::serialization_helpers::SArc;
use sapio_base::Clause;
use std::cmp::Reverse;

use std::collections::{BTreeMap, BTreeSet};

use std::sync::Arc;
mod cache;
use cache::*;
/// Used to prevent unintended callers to internal_clone.
pub struct InternalCompilerTag {
    _secret: (),
}

/// private::ImplSeal prevents anyone from implementing Compilable except by
/// implementing Contract.
mod private {
    pub trait ImplSeal {}
    /// Allow Contract to implement Compile
    impl ImplSeal for super::Compiled {}
    impl ImplSeal for bitcoin::XOnlyPublicKey {}
    impl<'a, C> ImplSeal for C where C: super::AnyContract {}
}
/// Compilable is a trait for anything which can be compiled
pub trait Compilable: private::ImplSeal {
    /// Compile a compilable object returning errors, if any.
    fn compile(&self, ctx: Context) -> Result<Compiled, CompilationError>;
}

/// Implements a basic identity
impl Compilable for Compiled {
    fn compile(&self, _ctx: Context) -> Result<Compiled, CompilationError> {
        Ok(self.clone())
    }
}

impl Compilable for bitcoin::XOnlyPublicKey {
    // TODO: Taproot; make infallible API
    fn compile(&self, ctx: Context) -> Result<Compiled, CompilationError> {
        let addr = bitcoin::Address::p2tr_tweaked(
            TweakedPublicKey::dangerous_assume_tweaked(self.clone()),
            ctx.network,
        );
        let mut amt = AmountRange::new();
        amt.update_range(ctx.funds());
        Ok(Compiled::from_address(addr, Some(amt)))
    }
}

#[derive(PartialEq, Eq)]
enum Nullable {
    Yes,
    No,
}

fn compute_all_effects<C, A: Default>(
    mut top_effect_ctx: Context,
    self_ref: &C,
    func: &dyn CallableAsFoF<C, A>,
) -> TxTmplIt {
    let default_applied_effect_ctx = top_effect_ctx.derive(PathFragment::DefaultEffect)?;
    let def = func.call(self_ref, default_applied_effect_ctx, Default::default());
    if func.web_api() {
        def
    } else {
        let mut applied_effects_ctx = top_effect_ctx.derive(PathFragment::Effects)?;
        top_effect_ctx
            .get_effects(InternalCompilerTag { _secret: () })
            .get_value(top_effect_ctx.path())
            // always gets the default expansion, but will also attempt
            // operating with the effects passed in through the Context Object.
            .fold(def, |a: TxTmplIt, (k, arg)| -> TxTmplIt {
                let v = a?;
                let c = applied_effects_ctx
                    .derive(PathFragment::Named(SArc(k.clone())))
                    .expect("Must be a valid derivation or internal invariant not held");
                let w = func.call_json(self_ref, c, arg.clone())?;
                Ok(Box::new(v.chain(w)))
            })
    }
}

struct Renamer {
    used_names: BTreeSet<String>,
}

impl Renamer {
    fn new() -> Self {
        Renamer {
            used_names: Default::default(),
        }
    }
    fn get_name(&mut self, a: &String) -> String {
        let count = 0u64;
        let mut name: String = a.clone();
        loop {
            if self.used_names.insert(name.clone()) {
                return name;
            } else {
                name = format!("{}_renamed_{}", a, count);
            }
        }
    }
}

impl<'a, T> Compilable for T
where
    T: AnyContract + 'a,
    T::Ref: 'a,
{
    /// The main Compilation Logic for a Contract.
    /// TODO: Better Document Semantics
    fn compile(&self, mut ctx: Context) -> Result<Compiled, CompilationError> {
        let self_ref = self.get_inner_ref();
        let mut guard_clauses = GuardCache::new();
        let dummy_root = Arc::new(ReversePath::from(PathFragment::Root));

        // The below maps track metadata that is useful for consumers / verification.
        // track transactions that are *guaranteed* via CTV
        let mut comitted_txns = BTreeMap::new();
        // All other transactions
        let mut other_txns = BTreeMap::new();

        // the min and max amount of funds spendable in the transactions
        let mut amount_range = AmountRange::new();

        // amount ensuring that the funds required don't get tweaked
        // during recompilation passes
        // TODO: Maybe do not just cloned?
        let amount_range_ctx = ctx.derive(PathFragment::Cloned)?;
        let ensured_amount = self.ensure_amount(amount_range_ctx)?;
        amount_range.update_range(ensured_amount);

        // The code for then_fns and finish_or_fns is very similar, differing
        // only in that then_fns have a CTV enforcing the contract and
        // finish_or_fns do not. We can lazily chain iterators to process them
        // in a row.
        //
        // we need a unique context for each.
        let mut action_ctx = ctx.derive(PathFragment::Action)?;
        let mut renamer = Renamer::new();
        let (mut continue_apis, clause_accumulator): (
            BTreeMap<SArc<EffectPath>, ContinuationPoint>,
            Vec<Vec<Miniscript<XOnlyPublicKey, Tap>>>,
        ) = self
            .then_fns()
            .iter()
            .filter_map(|func| func())
            // We currently need to allocate for the the Callable as a
            // trait object since it only exists temporarily.
            // TODO: Without allocations?
            .map(|x| -> Box<dyn CallableAsFoF<_, _>> { Box::new(x) })
            .chain(self.finish_or_fns().iter().filter_map(|func| func()))
            // TOOD: What is flat map doing here?
            .flat_map(|mut x| {
                let new_name = Arc::new(renamer.get_name(x.get_name().as_ref()));
                x.rename(new_name.clone());
                let name = PathFragment::Named(SArc(new_name));
                action_ctx.derive(name).map(|p| (p, x))
            })
            // flat_map will discard any
            // skippable / never branches here
            .flat_map(|(mut f_ctx, func)| {
                let mut this_ctx = f_ctx
                    // this should always be Ok(_)
                    .derive(PathFragment::CondCompIf)
                    .expect("Must be a valid derivation or internal invariant not held");
                match CCILWrapper(func.get_conditional_compile_if())
                    .assemble(self_ref, &mut this_ctx)
                {
                    // Throw errors
                    ConditionalCompileType::Fail(errors) => {
                        Some(Err(CompilationError::ConditionalCompilationFailed(errors)))
                    }
                    // Non nullable
                    ConditionalCompileType::Required | ConditionalCompileType::NoConstraint => {
                        Some(Ok((f_ctx, func, Nullable::No)))
                    }
                    // Nullable
                    ConditionalCompileType::Nullable => Some(Ok((f_ctx, func, Nullable::Yes))),
                    // Drop these
                    ConditionalCompileType::Skippable | ConditionalCompileType::Never => None,
                }
            })
            .map(|r| {
                let (mut f_ctx, func, nullability) = r?;
                let gctx = f_ctx.derive(PathFragment::Guard)?;
                // TODO: Suggested path frag?
                let guards = create_guards(self_ref, gctx, func.get_guard(), &mut guard_clauses);
                let effect_ctx = f_ctx.derive(if func.get_returned_txtmpls_modify_guards() {
                    PathFragment::Next
                } else {
                    PathFragment::Suggested
                })?;
                let effect_path = effect_ctx.path().clone();
                let transactions = compute_all_effects(effect_ctx, self_ref, func.as_ref());
                // If no guards and not CTV, then nothing gets added (not
                // interpreted as Trivial True)
                //   - If CTV and no guards, just CTV added.
                //   - If CTV and guards, CTV & guards added.
                // it would be an error if any of r_txtmpls is an error
                // instead of just an empty iterator.
                let txtmpl_clauses = transactions?
                    .map(|r_txtmpl| {
                        let txtmpl = r_txtmpl?;
                        let h = txtmpl.hash();
                        amount_range.update_range(txtmpl.max);
                        // Add the addition guards to these clauses
                        let txtmpl = if func.get_returned_txtmpls_modify_guards() {
                            &mut comitted_txns
                        } else {
                            &mut other_txns
                        }
                        .entry(h)
                        .or_insert(txtmpl);
                        let extractor = func.get_extract_clause_from_txtmpl();
                        (extractor)(&txtmpl, &ctx)
                    })
                    // Drop None values
                    .filter_map(|s| s.transpose())
                    // Forces any error to abort the whole thing
                    .collect::<Result<Vec<Clause>, CompilationError>>()?;

                let dummy = (
                    SArc(dummy_root.clone()),
                    ContinuationPoint::at(None, dummy_root.clone()),
                );
                // N.B. the order of the matches below is significant
                if func.get_returned_txtmpls_modify_guards() {
                    match (nullability, txtmpl_clauses.len(), guards) {
                        // This is a nullable branch without any proposed
                        // transactions.
                        // Therefore, mark this branch dead.
                        (Nullable::Yes, 0, _) => Ok((dummy, vec![])),
                        // Error if we expect CTV, returned some templates, but our guard
                        // was unsatisfiable, irrespective of nullability. This is because
                        // the behavior should be captured through a compile_if if it is
                        // intended.
                        (_, n, Clause::Unsatisfiable) if n > 0 => {
                            // TODO: Turn into a warning that the intended
                            // behavior should be to compile_if
                            Err(CompilationError::MissingTemplates)
                        }
                        // Error if 0 templates return and we don't want to be nullable
                        (Nullable::No, 0, _) => Err(CompilationError::MissingTemplates),
                        // If the guard is trivial, return the hashes standalone
                        (_, _, Clause::Trivial) => Ok((
                            dummy,
                            txtmpl_clauses
                                .into_iter()
                                .map(|policy| {
                                    policy.compile().map_err(Into::<CompilationError>::into)
                                })
                                .collect::<Result<Vec<_>, _>>()?,
                        )),
                        // If the guard is non-trivial, zip it to each hash
                        // TODO: Arc in miniscript to dedup memory?
                        //       This could be Clause::Shared(x) or something...
                        (_, _, guards) => Ok((
                            dummy,
                            txtmpl_clauses
                                .into_iter()
                                // extra_guards will contain any CTV
                                .map(|extra_guards| {
                                    Clause::And(vec![guards.clone(), extra_guards])
                                        .compile()
                                        .map_err(Into::<CompilationError>::into)
                                })
                                .collect::<Result<Vec<_>, _>>()?,
                        )),
                    }
                } else {
                    Ok((
                        (
                            SArc(effect_path.clone()),
                            ContinuationPoint::at(func.get_schema().clone(), effect_path),
                        ),
                        vec![guards.compile().map_err(Into::<CompilationError>::into)?],
                    ))
                }
            })
            .collect::<Result<Vec<(_, Vec<Miniscript<XOnlyPublicKey, Tap>>)>, CompilationError>>()?
            .into_iter()
            .unzip();

        // All of the CTV Branches add the dummy_root path,
        // so we remove it here.
        continue_apis.remove(&SArc(dummy_root.clone()));

        let branches: Vec<Miniscript<XOnlyPublicKey, Tap>> = {
            let mut finish_fns_ctx = ctx.derive(PathFragment::FinishFn)?;
            // Compute all finish_functions at this level, caching if requested.
            self.finish_fns()
                .iter()
                // note that this zip with would loop forever if there were to be a bug here
                .zip(
                    (0..)
                        .filter_map(|i| finish_fns_ctx.derive(PathFragment::Branch(i as u64)).ok()),
                )
                .filter_map(|(func, c)| guard_clauses.get(self_ref, *func, c))
                .map(|policy| policy.compile().map_err(Into::<CompilationError>::into))
                .chain(clause_accumulator.into_iter().flatten().map(Ok))
                .collect::<Result<Vec<_>, _>>()?
        };
        // TODO: Pick a better branch that is guaranteed to work!
        let some_key = pick_key_from_miniscripts(branches.iter());
        // Don't remove the key from the scripts in case it was bogus
        let tree = branches_to_tree(branches);
        let descriptor = Descriptor::Tr(descriptor::Tr::new(some_key, tree)?);
        let estimated_max_size = descriptor.max_satisfaction_weight()?;
        let address = descriptor.address(ctx.network)?.into();
        let descriptor = Some(descriptor.into());
        let root_path = SArc(ctx.path().clone());

        let failed_estimate = comitted_txns.values().any(|a| {
            // witness space not scaled
            let tx_size = a.tx.get_weight() + estimated_max_size;
            let fees = amount_range.max() - a.total_amount();
            a.min_feerate_sats_vbyte
                .map(|m| fees.as_sat() < (m.as_sat() * tx_size as u64))
                == Some(false)
        });
        if failed_estimate {
            Err(CompilationError::MinFeerateError)
        } else {
            let metadata_ctx = ctx.derive(PathFragment::Metadata)?;
            Ok(Compiled {
                ctv_to_tx: comitted_txns,
                suggested_txs: other_txns,
                continue_apis,
                root_path,
                address,
                descriptor,
                amount_range,
                metadata: self.metadata(metadata_ctx)?,
            })
        }
    }
}

/// picks a key from an iter of miniscripts, or returns a static default key
fn pick_key_from_miniscripts<'a, I: Iterator<Item = &'a Miniscript<XOnlyPublicKey, Tap>>>(
    branches: I,
) -> XOnlyPublicKey {
    branches
        .filter_map(|f| {
            if let Terminal::Check(check) = &f.node {
                if let Terminal::PkK(k) = &check.node {
                    return Some(k.clone());
                }
            }
            None
        })
        .next()
        .map(|x| bitcoin::util::schnorr::UntweakedPublicKey::from(x))
        .unwrap_or(
            XOnlyPublicKey::from_slice(&Sha256::hash(&[1u8; 32]).into_inner()).expect("constant"),
        )
}

/// Convert the branches into a heap for taproot tree consumption
fn branches_to_tree(
    branches: Vec<Miniscript<XOnlyPublicKey, Tap>>,
) -> Option<TapTree<XOnlyPublicKey>> {
    let mut scripts: BinaryHeap<(Reverse<u64>, TapTree<XOnlyPublicKey>)> = branches
        .into_iter()
        .map(|b| (Reverse(1), TapTree::Leaf(Arc::new(b))))
        .collect();
    while scripts.len() > 1 {
        let (w1, v1) = scripts.pop().unwrap();
        let (w2, v2) = scripts.pop().unwrap();
        scripts.push((
            Reverse(w1.0.saturating_add(w2.0)),
            TapTree::Tree(Arc::new(v1), Arc::new(v2)),
        ));
    }
    scripts.pop().map(|v| v.1)
}
