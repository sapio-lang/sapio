// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The primary compilation traits and types
use super::actions::conditional_compile;
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
use bitcoin::blockdata::opcodes::all::OP_VERIFY;
use bitcoin::hashes::hex::FromHex;
use bitcoin::schnorr::TweakedPublicKey;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::util::taproot::LeafVersion;
use bitcoin::util::taproot::TaprootBuilder;
use bitcoin::Address;
use bitcoin::Script;
use bitcoin::XOnlyPublicKey;
use miniscript::*;
use sapio_base::effects::EffectDB;
use sapio_base::effects::EffectPath;
use sapio_base::effects::PathFragment;
use sapio_base::miniscript;
use sapio_base::miniscript::miniscript::decode;
use sapio_base::serialization_helpers::SArc;
use sapio_base::Clause;
use serde::Deserialize;
use std::borrow::BorrowMut;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
mod cache;
mod util;
use cache::*;
use util::*;
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
            TweakedPublicKey::dangerous_assume_tweaked(*self),
            ctx.network,
        );
        let mut amt = AmountRange::new();
        amt.update_range(ctx.funds());
        Ok(Compiled::from_address(addr, Some(amt)))
    }
}

#[derive(PartialEq, Eq, Debug)]
enum Nullable {
    Yes,
    No,
}

const UNIQUE_DERIVE_PANIC_MSG: &str = "Must be a valid derivation or internal invariant not held";
fn create_state_transition_iterator<C, A: Default>(
    mut top_effect_ctx: Context,
    self_ref: &C,
    state_transition: &dyn CallableAsFoF<C, A>,
) -> TxTmplIt {
    let default_applied_effect_ctx = top_effect_ctx.derive(PathFragment::DefaultEffect)?;
    let def = state_transition.call(self_ref, default_applied_effect_ctx, Default::default())?;
    if !state_transition.web_api() {
        return Ok(def);
    }
    let mut applied_effects_ctx = top_effect_ctx.derive(PathFragment::Effects)?;
    let r = top_effect_ctx
        .get_effects(InternalCompilerTag { _secret: () })
        .get_value(top_effect_ctx.path())
        // always gets the default expansion, but will also attempt
        // operating with the effects passed in through the Context Object.
        .try_fold(def, |a, (k, arg)| -> TxTmplIt {
            let v = a;
            let c = applied_effects_ctx
                .derive(PathFragment::Named(SArc(k.clone())))
                .expect(UNIQUE_DERIVE_PANIC_MSG);
            let w = state_transition.call_json(self_ref, c, arg.clone())?;
            Ok(Box::new(v.chain(w)))
        });
    r
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

#[derive(Default)]
struct ContinueAPIs {
    inner: BTreeMap<SArc<EffectPath>, ContinuationPoint>,
}

type ContinueAPIEntry = Option<(SArc<EffectPath>, ContinuationPoint)>;

impl Extend<ContinueAPIEntry> for ContinueAPIs {
    fn extend<T>(&mut self, iter: T)
    where
        T: IntoIterator<Item = ContinueAPIEntry>,
    {
        self.inner.extend(iter.into_iter().flatten())
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
        let mut script_precondition_compilation_cache = GuardCache::new();

        // The below maps track metadata that is useful for consumers / verification.
        // track transactions that are *guaranteed* via CTV
        let mut committed_txn_map = BTreeMap::new();
        // All other transactions
        let mut uncommitted_txn_map = BTreeMap::new();

        // the min and max amount of funds spendable in the transactions
        let mut required_amount_range = AmountRange::new();

        // amount ensuring that the funds required don't get tweaked
        // during recompilation passes
        // TODO: Maybe do not just cloned?
        let amount_range_ctx = ctx.derive(PathFragment::Cloned)?;
        let ensured_amount = self.ensure_amount(amount_range_ctx)?;
        required_amount_range.update_range(ensured_amount);

        // The code for then_fns and finish_or_fns is very similar, differing
        // only in that then_fns have a CTV enforcing the contract and
        // finish_or_fns do not. We can lazily chain iterators to process them
        // in a row.
        //
        // we need a unique context for each.
        let mut action_ctx = ctx.derive(PathFragment::Action)?;
        let mut renamer = Renamer::new();
        let all_state_transition_functions = self
            .then_fns()
            .iter()
            .filter_map(|state_transition| state_transition())
            // We currently need to allocate for the the Callable as a
            // trait object since it only exists temporarily.
            // TODO: Without allocations?
            .map(|x| -> Box<dyn CallableAsFoF<_, _>> { Box::new(x) })
            .chain(
                self.finish_or_fns()
                    .iter()
                    .filter_map(|state_transition| state_transition()),
            );
        let rename_state_transitions_uniquely =
            all_state_transition_functions.map(|mut state_transition| {
                let new_name = Arc::new(renamer.get_name(state_transition.get_name().as_ref()));
                state_transition.rename(new_name.clone());
                let name = PathFragment::Named(SArc(new_name));
                let state_transition_context =
                    action_ctx.derive(name).expect(UNIQUE_DERIVE_PANIC_MSG);
                (state_transition_context, state_transition)
            });
        // flat_map will discard any
        // skippable / never branches here
        let filtered_conditional_compilation = rename_state_transitions_uniquely.flat_map(
            |(mut state_transition_context, state_transition)| {
                let mut this_ctx = state_transition_context
                    // this should always be Ok(_)
                    .derive(PathFragment::CondCompIf)
                    .expect(UNIQUE_DERIVE_PANIC_MSG);
                match CCILWrapper(state_transition.get_conditional_compile_if())
                    .assemble(self_ref, &mut this_ctx)
                {
                    // Throw errors
                    ConditionalCompileType::Fail(errors) => {
                        Some(Err(CompilationError::ConditionalCompilationFailed(errors)))
                    }
                    // Non nullable
                    ConditionalCompileType::Required | ConditionalCompileType::NoConstraint => {
                        Some(Ok((
                            state_transition_context,
                            state_transition,
                            Nullable::No,
                        )))
                    }
                    // Nullable
                    ConditionalCompileType::Nullable => Some(Ok((
                        state_transition_context,
                        state_transition,
                        Nullable::Yes,
                    ))),
                    // Drop these
                    ConditionalCompileType::Skippable | ConditionalCompileType::Never => None,
                }
            },
        );
        let all_script_predicates = filtered_conditional_compilation
            .map(|r| {
                let (mut state_transition_context, state_transition, nullability_enabled) = r?;
                let script_precondition_context =
                    state_transition_context.derive(PathFragment::Guard)?;
                let interactive_metadata_context =
                    state_transition_context.derive(PathFragment::Metadata)?;
                // TODO: Suggested path frag?
                let (script_precondition, script_precondition_metadata) =
                    get_script_preconditions_for(
                        self_ref,
                        script_precondition_context,
                        state_transition.get_guard(),
                        &mut script_precondition_compilation_cache,
                    )?;
                // If the txtmpls that are returned from this state transition
                // modify guards, then it is a CTV based state transition and it should be
                // labelled as "Next"
                let effect_ctx = state_transition_context.derive(
                    if state_transition.returned_transaction_templates_can_modify_parent_script() {
                        PathFragment::Next
                    } else {
                        PathFragment::Suggested
                    },
                )?;
                let continuation_path = effect_ctx.path().clone();
                let transactions = create_state_transition_iterator(
                    effect_ctx,
                    self_ref,
                    state_transition.as_ref(),
                )?;
                // If no guards and not CTV, then nothing gets added (not
                // interpreted as Trivial True)
                //   - If CTV and no guards, just CTV added.
                //   - If CTV and guards, CTV & guards added.
                // it would be an error if any of r_txtmpls is an error
                // instead of just an empty iterator.
                let transaction_script_preconditions = transactions
                    .map(|tx_template_or_error| {
                        let tx_template = tx_template_or_error?;
                        let tx_checktemplateverify_hash = tx_template.hash();
                        required_amount_range.update_range(tx_template.max);
                        // Add the addition guards to these clauses
                        let stored_tx_template = if state_transition
                            .returned_transaction_templates_can_modify_parent_script()
                        {
                            &mut committed_txn_map
                        } else {
                            &mut uncommitted_txn_map
                        }
                        .entry(tx_checktemplateverify_hash)
                        .or_insert(tx_template);

                        state_transition.extract_script_preconditions_from_transaction_template()(
                            stored_tx_template,
                            &ctx,
                        )
                    })
                    // Drop None values
                    .filter_map(|s| s.transpose())
                    // Forces any error to abort the whole thing
                    .collect::<Result<Vec<Clause>, CompilationError>>()?;

                // N.B. the order of the matches below is significant
                Ok(
                    if state_transition.returned_transaction_templates_can_modify_parent_script() {
                        let r = (
                            None,
                            combine_txtmpls(
                                nullability_enabled,
                                transaction_script_preconditions,
                                script_precondition,
                            )?,
                            script_precondition_metadata,
                        );
                        r
                    } else {
                        let simps =
                            state_transition.gen_simps(self_ref, interactive_metadata_context)?;
                        let continuation_point = simps.iter().try_fold(
                            ContinuationPoint::at(
                                state_transition.get_schema().clone(),
                                continuation_path.clone(),
                            ),
                            |c, simp| c.add_simp(simp.as_ref()),
                        )?;
                        let v = sapio_base::Clause::generate_leafs(script_precondition)?;
                        (
                            Some((SArc(continuation_path), continuation_point)),
                            v,
                            script_precondition_metadata,
                        )
                    },
                )
            })
            .collect::<Result<Vec<(_, Vec<Script>, _)>, CompilationError>>()?;

        let mut continue_apis = ContinueAPIs::default();
        let mut clause_accumulator = vec![];
        let mut all_guard_simps: BTreeMap<Clause, GuardSimps> = Default::default();
        for (v, b, c) in all_script_predicates {
            continue_apis.extend(std::iter::once(v));
            clause_accumulator.push(b);
            for (pol, mut simps) in c {
                all_guard_simps.entry(pol).or_default().append(&mut simps)
            }
        }
        for guard_simps in all_guard_simps.values_mut() {
            guard_simps.sort_by_key(|k| k as *const _ as usize);
            guard_simps.dedup_by(|a, b| std::ptr::eq(a, b))
        }

        let branches: Vec<Script> = {
            let mut finish_fns_ctx = ctx.derive(PathFragment::FinishFn)?;
            // Compute all finish_functions at this level, caching if requested.
            let guards = self
                .finish_fns()
                .iter()
                // note that this zip with would loop forever if there were to be a bug here
                .zip((0..).filter_map(|i| {
                    let mut new = finish_fns_ctx.derive(PathFragment::Branch(i as u64)).ok()?;
                    let simp = new.derive(PathFragment::Metadata).ok()?;
                    Some((new, simp))
                }))
                .filter_map(|(state_transition, (c, simp_c))| {
                    script_precondition_compilation_cache
                        .get(self_ref, *state_transition, c, simp_c)
                        .transpose()
                })
                .collect::<Result<Vec<_>, _>>()?;
            let all_g = guards
                .into_iter()
                .map(|(policy, _m)| Clause::generate_leafs(policy))
                .collect::<Result<Vec<_>, _>>()?;

            all_g
                .into_iter()
                .chain(clause_accumulator.into_iter())
                .flatten()
                .collect()
        };

        let decode = Miniscript::<XOnlyPublicKey, Tap>::parse;
        // TODO: Pick a better branch that is guaranteed to work!
        let decoded_branches: Vec<_> = branches.iter().filter_map(|s| decode(s).ok()).collect();
        let some_key = pick_key_from_miniscripts(decoded_branches.iter());
        // Don't remove the key from the scripts in case it was bogus
        let huffman_tree =
            TaprootBuilder::with_huffman_tree(branches.iter().map(|x| (1, x.clone())))
                .map_err(|_| CompilationError::TaprootBuilderError)?;

        let secp = Secp256k1::verification_only();
        let taproot_spend_info = huffman_tree
            .finalize(&secp, some_key)
            .map_err(|_| CompilationError::TaprootBuilderError)?;
        let mut estimated_max_size = 0;
        for code in decoded_branches {
            if let Some(blk) =
                // TODO: drop the re-encode
                taproot_spend_info.control_block(&(code.encode(), LeafVersion::TapScript))
            {
                estimated_max_size = std::cmp::max(
                    blk.size() + code.max_satisfaction_size()?,
                    estimated_max_size,
                );
            }
        }
        let tr_address = Address::p2tr_tweaked(taproot_spend_info.output_key(), ctx.network);

        // TODO: Convert into an address instead of keeping descriptor,
        // hot-fix workaround
        let address = tr_address.into();
        let root_path = SArc(ctx.path().clone());

        let failed_estimate = committed_txn_map.values().any(|a| {
            // witness space not scaled
            let tx_size = a.tx.get_weight() + estimated_max_size;
            let fees = required_amount_range.max() - a.total_amount();
            a.min_feerate_sats_vbyte
                .map(|m| fees.as_sat() < (m.as_sat() * tx_size as u64))
                == Some(false)
        });
        if failed_estimate {
            Err(CompilationError::MinFeerateError)
        } else {
            let metadata_ctx = ctx.derive(PathFragment::Metadata)?;
            let metadata = self
                .metadata(metadata_ctx)?
                .add_guard_simps(all_guard_simps)?;
            let taproot_spend_info = Some(taproot_spend_info);
            Ok(Compiled {
                ctv_to_tx: committed_txn_map,
                suggested_txs: uncommitted_txn_map,
                continue_apis: continue_apis.inner,
                root_path,
                address,
                taproot_spend_info,
                amount_range: required_amount_range,
                metadata,
            })
        }
    }
}

fn combine_txtmpls(
    nullability: Nullable,
    unique_transaction_clauses: Vec<Clause>,
    guards: Clause,
) -> Result<Vec<Script>, CompilationError> {
    match (nullability, unique_transaction_clauses.len(), guards) {
        // This is a nullable branch without any proposed
        // transactions.
        // Therefore, mark this branch dead.
        (Nullable::Yes, 0, _) => Ok(vec![]),
        // Error if we expect CTV, returned some templates, but our guard
        // was unsatisfiable, irrespective of nullability. This is because
        // the behavior should be captured through a compile_if if it is
        // intended.
        (_, n, Clause::Script(s))
            if n > 0 && matches!(s.as_bytes(), sapio_base::consts::FALSE_PATTERN) =>
        {
            // TODO: Turn into a warning that the intended
            // behavior should be to compile_if
            Err(CompilationError::MissingTemplates)
        }
        // Error if 0 templates return and we don't want to be nullable
        (Nullable::No, 0, _) => Err(CompilationError::MissingTemplates),
        // If the guard is trivial, return the hashes standalone
        (_, _, Clause::Script(s)) if matches!(s.as_bytes(), sapio_base::consts::TRUE_PATTERN) => {
            let mut acc = vec![];
            for unique_tx in unique_transaction_clauses {
                let leafs = sapio_base::Clause::generate_leafs(unique_tx)?;
                acc.extend(leafs);
            }
            Ok(acc)
        }
        // If the guard is non-trivial, zip it to each hash
        // TODO: Arc in miniscript to dedup memory?
        //       This could be Clause::Shared(x) or something...
        (_, _, guards) => {
            let mut acc = vec![];
            for unique_tx in unique_transaction_clauses {
                let leafs = sapio_base::Clause::generate_leafs(Clause::And(
                    guards.clone().wrap(),
                    unique_tx.wrap(),
                ))
                .map_err(Into::<CompilationError>::into)?;
                acc.extend(leafs);
            }
            Ok(acc)
        }
    }
}
