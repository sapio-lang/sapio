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
use bitcoin::schnorr::TweakedPublicKey;
use bitcoin::XOnlyPublicKey;
use miniscript::*;
use sapio_base::effects::EffectDB;
use sapio_base::effects::EffectPath;
use sapio_base::effects::PathFragment;
use sapio_base::miniscript;
use sapio_base::serialization_helpers::SArc;
use sapio_base::Clause;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
mod cache;
mod util;
use cache::*;
use util::*;
/// Grants the compiler access to effects at the current compilation path.
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

fn compute_all_effects<C, A: Default>(
    mut top_effect_ctx: Context,
    self_ref: &C,
    func: &dyn CallableAsFoF<C, A>,
) -> TxTmplIt {
    // Reject malformed names before any continuation callback can observe an
    // effect path that would change meaning when serialized.
    if func.web_api() {
        for (name, _) in top_effect_ctx
            .get_effects(InternalCompilerTag { _secret: () })
            .get_value(top_effect_ctx.path())
        {
            if !matches!(
                PathFragment::try_from(name.clone())?,
                PathFragment::Named(_)
            ) {
                return Err(CompilationError::InvalidPathName);
            }
        }
    }
    let default_applied_effect_ctx = top_effect_ctx.derive(PathFragment::DefaultEffect)?;
    let def = func.call(self_ref, default_applied_effect_ctx, Default::default())?;
    if !func.web_api() {
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
            let c = applied_effects_ctx.derive_str(k.clone())?;
            let w = func.call_json(self_ref, c, arg.clone())?;
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
        let mut count = 0u64;
        let mut name: String = a.clone();
        loop {
            if self.used_names.insert(name.clone()) {
                return name;
            } else {
                name = format!("{}_renamed_{}", a, count);
                count += 1;
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

        // Extract each declared action's policy before deduplicating its
        // transaction payload. Equal CTV hashes do not imply equal guards.
        let mut action_ctx = ctx.derive(PathFragment::Action)?;
        let mut renamer = Renamer::new();
        let mut continue_apis = BTreeMap::new();
        let mut action_branches = vec![];
        let mut all_guard_simps: BTreeMap<Clause, GuardSimps> = BTreeMap::new();
        let then_fns = self.then_fns();
        let finish_or_fns = self.finish_or_fns();
        let actions = then_fns
            .iter()
            .filter_map(|factory| factory())
            .map(|action| -> Box<dyn CallableAsFoF<_, _>> { Box::new(action) })
            .chain(finish_or_fns.iter().filter_map(|factory| factory()));
        for mut action in actions {
            // Validate the source name before renaming: reserved fragments must
            // never acquire a different meaning after a JSON round trip.
            let original_name: PathFragment = action.get_name().clone().try_into()?;
            if !matches!(original_name, PathFragment::Named(_)) {
                return Err(CompilationError::InvalidPathName);
            }
            let name = Arc::new(renamer.get_name(action.get_name()));
            action.rename(name.clone());
            let mut action_context = action_ctx.derive_str(name)?;
            let mut condition_context = action_context.derive(PathFragment::CondCompIf)?;
            let nullability = match CCILWrapper(action.get_conditional_compile_if())
                .assemble(self_ref, &mut condition_context)?
            {
                ConditionalCompileType::Fail(errors) => {
                    return Err(CompilationError::ConditionalCompilationFailed(errors));
                }
                ConditionalCompileType::Required | ConditionalCompileType::NoConstraint => {
                    Nullable::No
                }
                ConditionalCompileType::Nullable => Nullable::Yes,
                ConditionalCompileType::Skippable | ConditionalCompileType::Never => continue,
            };
            let guard_context = action_context.derive(PathFragment::Guard)?;
            let metadata_context = action_context.derive(PathFragment::Metadata)?;
            let (guards, guard_metadata) = create_guards(
                self_ref,
                guard_context,
                action.get_guard(),
                &mut guard_clauses,
            )?;
            let committed = action.get_returned_txtmpls_modify_guards();
            let effect_context = action_context.derive(if committed {
                PathFragment::Next
            } else {
                PathFragment::Suggested
            })?;
            let effect_path = effect_context.path().clone();
            let mut template_clauses = vec![];
            for template in compute_all_effects(effect_context, self_ref, action.as_ref())? {
                let mut template = template?;
                // This also rejects forbidden guards on every suggested
                // template, including duplicates of an earlier valid template.
                let clause = (action.get_extract_clause_from_txtmpl())(&template, &ctx)?;
                amount_range.update_range(template.required_input_amount);
                if committed {
                    template.guards = policy_as_guards(conjoin_guards(
                        std::iter::once(&guards).chain(template.guards.iter()),
                    ));
                }
                insert_template(
                    if committed {
                        &mut comitted_txns
                    } else {
                        &mut other_txns
                    },
                    template,
                    &effect_path,
                )?;
                if let Some(clause) = clause {
                    template_clauses.push(clause);
                }
            }
            action_branches.extend(if committed {
                combine_txtmpls(nullability, template_clauses, guards)?
            } else {
                let mut continuation =
                    ContinuationPoint::at(action.get_schema().clone(), effect_path.clone());
                for simp in action.gen_simps(self_ref, metadata_context)? {
                    continuation = continuation.add_simp(simp.as_ref())?;
                }
                continue_apis.insert(SArc(effect_path), continuation);
                optimizer_flatten_and_compile(guards)?
            });
            for (policy, mut simps) in guard_metadata {
                all_guard_simps
                    .entry(policy)
                    .or_default()
                    .append(&mut simps);
            }
        }

        let mut finish_context = ctx.derive(PathFragment::FinishFn)?;
        let mut branches = vec![];
        for (index, factory) in self.finish_fns().iter().enumerate() {
            let mut guard_context = finish_context.derive_num(index as u64)?;
            let metadata_context = guard_context.derive(PathFragment::Metadata)?;
            if let Some((policy, mut simps)) =
                guard_clauses.get(self_ref, *factory, guard_context, metadata_context)?
            {
                all_guard_simps
                    .entry(policy.clone())
                    .or_default()
                    .append(&mut simps);
                branches.extend(optimizer_flatten_and_compile(policy)?);
            }
        }
        branches.extend(action_branches);
        // TODO: Pick a better branch that is guaranteed to work!
        let some_key = pick_key_from_miniscripts(branches.iter());
        // Don't remove the key from the scripts in case it was bogus
        let tree = branches_to_tree(branches);
        let descriptor = Descriptor::Tr(descriptor::Tr::new(some_key, tree)?);
        let estimated_max_size = descriptor.max_satisfaction_weight()?;
        // TODO: Convert into an address instead of keeping descriptor,
        // hot-fix workaround
        let address = descriptor.clone().into();
        let descriptor = Some(descriptor.into());
        let root_path = SArc(ctx.path().clone());

        let failed_estimate = comitted_txns.values().any(|a| {
            let Some(rate) = a.min_feerate_sats_vbyte else {
                return false;
            };
            // Other inputs' satisfactions are unknown until binding. Do not
            // promise a minimum feerate without a bound on their weight.
            if a.tx.input.len() != 1 {
                return true;
            }
            let vsize = (a.tx.weight() + estimated_max_size + 2).div_ceil(4) as u64;
            // Only this template's reserved fees count. A larger funding
            // requirement in another branch cannot subsidize this spend.
            let fees = a.max.checked_sub(a.total_amount());
            match (fees, rate.as_sat().checked_mul(vsize)) {
                (Some(fees), Some(required)) => fees.as_sat() < required,
                _ => true,
            }
        });
        if failed_estimate {
            Err(CompilationError::MinFeerateError)
        } else {
            let metadata_ctx = ctx.derive(PathFragment::Metadata)?;
            let metadata = self
                .metadata(metadata_ctx)?
                .add_guard_simps(all_guard_simps)?;
            Ok(Compiled {
                ctv_to_tx: comitted_txns,
                suggested_txs: other_txns,
                continue_apis,
                root_path,
                address,
                descriptor,
                amount_range,
                metadata,
            })
        }
    }
}

pub(crate) fn conjoin_guards<'a>(guards: impl Iterator<Item = &'a Clause>) -> Clause {
    fn contains_inscription(guard: &Clause) -> bool {
        match guard {
            Clause::Inscribe(..) => true,
            Clause::And(guards) | Clause::Threshold(_, guards) => {
                guards.iter().any(contains_inscription)
            }
            Clause::Or(guards) => guards.iter().any(|(_, guard)| contains_inscription(guard)),
            _ => false,
        }
    }

    let mut combined = vec![];
    for guard in guards {
        if *guard == Clause::Unsatisfiable {
            return Clause::Unsatisfiable;
        }
        // Inscription envelopes are script effects. Neither their order nor
        // their multiplicity follows Boolean idempotence, including when they
        // occur below another policy node.
        if *guard != Clause::Trivial && (contains_inscription(guard) || !combined.contains(guard)) {
            combined.push(guard.clone());
        }
    }
    match combined.len() {
        0 => Clause::Trivial,
        1 => combined.pop().unwrap(),
        2 => Clause::And(combined),
        count => Clause::Threshold(count, combined),
    }
}

fn policy_as_guards(policy: Clause) -> Vec<Clause> {
    if policy == Clause::Trivial {
        vec![]
    } else {
        vec![policy]
    }
}

fn insert_template(
    templates: &mut BTreeMap<bitcoin::hashes::sha256::Hash, crate::template::Template>,
    template: crate::template::Template,
    path: &EffectPath,
) -> Result<(), CompilationError> {
    use std::collections::btree_map::Entry;
    let hash = template.hash();
    match templates.entry(hash) {
        Entry::Vacant(entry) => {
            entry.insert(template);
        }
        Entry::Occupied(mut entry) => {
            let existing = entry.get_mut();
            // A CTV hash commits to transaction fields, not funding budgets,
            // metadata or child continuation paths. None may be chosen by
            // whichever action happens to be visited first.
            macro_rules! same {
                ($($field:ident),+ $(,)?) => { $(
                    if existing.$field != template.$field {
                        return Err(CompilationError::ConflictingTemplate {
                            hash, at: path.clone(), field: stringify!($field),
                        });
                    }
                )+ };
            }
            same!(
                ctv_index,
                tx,
                max,
                required_input_amount,
                min_feerate_sats_vbyte,
                metadata_map_s2s,
                inputs,
                outputs
            );
            let alternatives = optimizer_flatten_policy(conjoin_guards(existing.guards.iter()))
                .into_iter()
                .chain(optimizer_flatten_policy(conjoin_guards(
                    template.guards.iter(),
                )))
                .collect::<BTreeSet<_>>();
            existing.guards = if alternatives.contains(&Clause::Trivial) {
                vec![]
            } else if alternatives.len() == 1 {
                alternatives.into_iter().collect()
            } else {
                vec![Clause::Threshold(1, alternatives.into_iter().collect())]
            };
        }
    }
    Ok(())
}

fn optimizer_flatten_and_compile(
    guards: policy::Concrete<XOnlyPublicKey>,
) -> Result<Vec<Miniscript<XOnlyPublicKey, Tap>>, CompilationError> {
    let v = optimizer_flatten_policy(guards)
        .into_iter()
        .map(|g| g.compile())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(v)
}

fn combine_txtmpls(
    nullability: Nullable,
    txtmpl_clauses: Vec<Clause>,
    guards: Clause,
) -> Result<Vec<Miniscript<XOnlyPublicKey, Tap>>, CompilationError> {
    match (nullability, txtmpl_clauses.len(), guards) {
        // This is a nullable branch without any proposed
        // transactions.
        // Therefore, mark this branch dead.
        (Nullable::Yes, 0, _) => Ok(vec![]),
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
        (_, _, Clause::Trivial) => {
            let r = Ok(txtmpl_clauses
                .into_iter()
                .map(|policy| policy.compile().map_err(Into::<CompilationError>::into))
                .collect::<Result<Vec<_>, _>>()?);
            r
        }
        // If the guard is non-trivial, zip it to each hash
        // TODO: Arc in miniscript to dedup memory?
        //       This could be Clause::Shared(x) or something...
        (_, _, guards) => Ok(txtmpl_clauses
            .into_iter()
            // extra_guards will contain any CTV
            .map(|extra_guards| {
                Clause::And(vec![guards.clone(), extra_guards])
                    .compile()
                    .map_err(Into::<CompilationError>::into)
            })
            .collect::<Result<Vec<_>, _>>()?),
    }
}

fn optimizer_flatten_policy(p: Clause) -> Vec<Clause> {
    match p {
        policy::Concrete::Or(v) => v
            .into_iter()
            .flat_map(|(_, b)| optimizer_flatten_policy(b))
            .collect(),
        policy::Concrete::Threshold(1, v) => {
            v.into_iter().flat_map(optimizer_flatten_policy).collect()
        }
        p => vec![p],
    }
}
