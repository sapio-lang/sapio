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
use crate::contract::object::CovenantRequirements;
use crate::contract::TxTmplIt;
use bitcoin::key::TweakedPublicKey;
use bitcoin::XOnlyPublicKey;
use miniscript::*;
use sapio_base::covenant::{Ctv, Emulatable};
use sapio_base::effects::EffectDB;
use sapio_base::effects::EffectPath;
use sapio_base::effects::PathFragment;
use sapio_base::miniscript;
use sapio_base::policy::{PolicyCompiler, ScriptPolicy};
use sapio_base::serialization_helpers::SArc;
use sapio_base::Clause;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
mod cache;
mod feasibility;
#[cfg(test)]
mod internal_key_tests;
mod script;
mod util;
mod validation;
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

/// Compile a policy that describes exactly one Taproot script leaf.
///
/// This uses the contract compiler's checked policy lowering without building
/// a contract or choosing an internal key. Empty or alternative policies are
/// rejected. Emulatable predicates must already be resolved using explicit
/// public lowering inputs; this helper does not choose an emulation mode.
pub fn compile_policy_leaf(
    policy: &(impl PolicyCompiler + ?Sized),
) -> Result<bitcoin::ScriptBuf, CompilationError> {
    let mut leaves = script::lower_script_policy(&policy.compile_policy()?)?;
    match leaves.len() {
        0 => Err(CompilationError::EmptyPolicy),
        1 => Ok(leaves.remove(0)),
        count => Err(CompilationError::Custom(
            format!("expected one policy leaf, found {count}").into(),
        )),
    }
}

/// Implements a basic identity
impl Compilable for Compiled {
    fn compile(&self, ctx: Context) -> Result<Compiled, CompilationError> {
        self.validate_for_lowering(ctx.lowering_plan())?;
        Ok(self.clone())
    }
}

impl Compilable for bitcoin::XOnlyPublicKey {
    // TODO: Taproot; make infallible API
    fn compile(&self, ctx: Context) -> Result<Compiled, CompilationError> {
        ctx.lowering_plan().validate()?;
        let addr = bitcoin::Address::p2tr_tweaked(
            TweakedPublicKey::dangerous_assume_tweaked(*self),
            ctx.network,
        );
        Ok(Compiled::from_address(addr, ctx.funds()))
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
        ctx.lowering_plan().validate()?;
        let pinned_internal_key = self.pinned_internal_key(&ctx)?;
        let mut covenant_requirements = CovenantRequirements {
            lowering: ctx.lowering_plan().clone(),
            predicates: BTreeSet::new(),
        };
        let self_ref = self.get_inner_ref();
        let mut guard_clauses = GuardCache::new();

        // The below maps track metadata that is useful for consumers / verification.
        // track transactions that are *guaranteed* via CTV
        let mut comitted_txns = BTreeMap::new();
        // All other transactions
        let mut other_txns = BTreeMap::new();

        // Preserve the declared input floor independently of the templates.
        // A finish-only contract can require funding without producing one.
        let funding_context = ctx.derive(PathFragment::Cloned)?;
        let mut required_input_amount = self.ensure_amount(funding_context)?;

        // Extract each declared action's policy before deduplicating its
        // transaction payload. Equal CTV hashes do not imply equal guards.
        let mut action_ctx = ctx.derive(PathFragment::Action)?;
        let mut renamer = Renamer::new();
        let mut continue_apis = BTreeMap::new();
        let mut branches = vec![];
        let mut branch_bytes = 0usize;
        let mut all_guard_simps: BTreeMap<ScriptPolicy, GuardSimps> = BTreeMap::new();
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
            let resolved_guards = script::resolve_emulation(&guards, &mut covenant_requirements)?;
            let committed = action.template_kind() == super::actions::TemplateKind::Covenant;
            let effect_context = action_context.derive(if committed {
                PathFragment::Next
            } else {
                PathFragment::Suggested
            })?;
            let effect_path = effect_context.path().clone();
            let mut produced_clause = false;
            for template in compute_all_effects(effect_context, self_ref, action.as_ref())? {
                let mut template = template?;
                for guard in &template.guards {
                    script::validate_source(guard)?;
                }
                // This also rejects forbidden guards on every suggested
                // template, including duplicates of an earlier valid template.
                let clause = if committed {
                    let source = ScriptPolicy::from(Emulatable(Ctv(template.hash())));
                    Some(script::resolve_emulation(
                        &conjoin_source(template.guards.iter().chain(std::iter::once(&source))),
                        &mut covenant_requirements,
                    )?)
                } else if template.guards.is_empty() {
                    None
                } else {
                    return Err(CompilationError::AdditionalGuardsNotAllowedHere);
                };
                if let Some(clause) = &clause {
                    script::validate_source(clause)?;
                    if committed
                        && (!source_policy_possible(&guards, &template.tx)
                            || template
                                .guards
                                .iter()
                                .any(|guard| !source_policy_possible(guard, &template.tx))
                            || !source_policy_possible(clause, &template.tx))
                    {
                        return Err(CompilationError::ImpossibleTemplate {
                            hash: template.hash(),
                            at: effect_path.as_ref().clone(),
                        });
                    }
                }
                required_input_amount = required_input_amount.max(template.required_input_amount);
                if committed {
                    template.guards = policy_as_guards(conjoin_source(
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
                    produced_clause = true;
                    if committed {
                        append_branches(
                            &mut branches,
                            &mut branch_bytes,
                            compile_branches(conjoin_source(
                                [&resolved_guards, &clause].into_iter(),
                            ))?,
                        )?;
                    }
                }
            }
            if committed {
                if !produced_clause && nullability == Nullable::No {
                    return Err(CompilationError::MissingTemplates);
                }
            } else {
                let mut continuation =
                    ContinuationPoint::at(action.get_schema().clone(), effect_path.clone());
                for simp in action.gen_simps(self_ref, metadata_context)? {
                    continuation = continuation.add_simp(simp.as_ref())?;
                }
                continue_apis.insert(SArc(effect_path), continuation);
                append_branches(
                    &mut branches,
                    &mut branch_bytes,
                    compile_branches(resolved_guards)?,
                )?;
            }
            for (policy, mut simps) in guard_metadata {
                all_guard_simps
                    .entry(policy)
                    .or_default()
                    .append(&mut simps);
            }
        }

        let mut finish_context = ctx.derive(PathFragment::FinishFn)?;
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
                let policy = script::resolve_emulation(&policy, &mut covenant_requirements)?;
                append_branches(&mut branches, &mut branch_bytes, compile_branches(policy)?)?;
            }
        }
        // Only a proven standalone Miniscript key may become a key-path spend.
        let some_key = if let Some(key) = pinned_internal_key {
            let matches = |branch: &CompiledBranch| matches!(branch, CompiledBranch::Miniscript(script) if bare_key(script) == Some(key));
            if !branches.iter().any(matches) {
                return Err(CompilationError::UnauthorizedInternalKey { key });
            }
            // The exact bare-key branch is now satisfied through the key path.
            // Constrained branches and opaque scripts retain their full leaves.
            branches.retain(|branch| !matches(branch));
            key
        } else {
            if branches.is_empty() {
                return Err(CompilationError::EmptyPolicy);
            }
            pick_key_from_miniscripts(branches.iter().filter_map(|branch| match branch {
                CompiledBranch::Miniscript(script) => Some(script),
                CompiledBranch::Script(_) => None,
            }))
        };
        let opaque = branches
            .iter()
            .any(|branch| matches!(branch, CompiledBranch::Script(_)));
        let (address, descriptor, estimated_max_size) = if opaque {
            let scripts = branches
                .into_iter()
                .map(|branch| match branch {
                    CompiledBranch::Miniscript(script) => script.encode(),
                    CompiledBranch::Script(script) => script,
                })
                .collect();
            let raw = crate::contract::object::RawTaproot::from_scripts(some_key, scripts)?;
            let address =
                bitcoin::Address::p2tr_tweaked(raw.spend_info().output_key(), ctx.network).into();
            (
                address,
                crate::contract::object::SupportedDescriptors::Taproot(raw),
                None,
            )
        } else {
            let native = branches
                .into_iter()
                .filter_map(|branch| match branch {
                    CompiledBranch::Miniscript(script) => Some(script),
                    CompiledBranch::Script(_) => None,
                })
                .collect();
            let tree = branches_to_tree(native);
            let descriptor = Descriptor::Tr(descriptor::Tr::new(some_key, tree)?);
            let weight = descriptor.max_weight_to_satisfy()?;
            (descriptor.clone().into(), descriptor.into(), Some(weight))
        };
        let descriptor = Some(descriptor);
        let root_path = SArc(ctx.path().clone());

        if estimated_max_size.is_none()
            && comitted_txns
                .values()
                .any(|t| t.min_feerate_sats_vbyte.is_some())
        {
            return Err(CompilationError::UnknownSatisfactionWeight);
        }

        let failed_estimate = comitted_txns.values().any(|a| {
            let Some(rate) = a.min_feerate_sats_vbyte else {
                return false;
            };
            // Other inputs' satisfactions are unknown until binding. Do not
            // promise a minimum feerate without a bound on their weight.
            if a.tx.input.len() != 1 {
                return true;
            }
            let Some(estimated_max_size) = estimated_max_size else {
                return true;
            };
            // The unsigned transaction has no Segwit serialization yet. Add
            // marker/flag and the empty witness count before the satisfaction
            // delta, which is measured against an empty Segwit input.
            let vsize =
                (a.tx.weight() + estimated_max_size + bitcoin::Weight::from_wu(3)).to_vbytes_ceil();
            // Only this template's reserved fees count. A larger funding
            // requirement in another branch cannot subsidize this spend.
            let fees = a.max.checked_sub(a.total_amount());
            match (fees, rate.to_sat().checked_mul(vsize)) {
                (Some(fees), Some(required)) => fees.to_sat() < required,
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
            let compiled = Compiled {
                covenant_requirements,
                ctv_to_tx: comitted_txns,
                suggested_txs: other_txns,
                continue_apis,
                root_path,
                address,
                descriptor,
                required_input_amount,
                metadata,
            };
            compiled.validate()?;
            Ok(compiled)
        }
    }
}

pub(crate) fn conjoin_guards<'a>(guards: impl Iterator<Item = &'a Clause>) -> Clause {
    fn contains_inscription(guard: &Clause) -> bool {
        match guard {
            Clause::Inscribe(..) => true,
            Clause::And(guards) => guards.iter().any(|guard| contains_inscription(guard)),
            Clause::Thresh(threshold) => threshold.iter().any(|guard| contains_inscription(guard)),
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
        2 => Clause::And(combined.into_iter().map(Arc::new).collect()),
        _ => Clause::Thresh(miniscript::Threshold::and_n(
            combined.into_iter().map(Arc::new).collect(),
        )),
    }
}

/// Compose policy sources without lowering a raw fragment prematurely.
pub(crate) fn conjoin_source<'a>(policies: impl Iterator<Item = &'a ScriptPolicy>) -> ScriptPolicy {
    let policies: Vec<_> = policies.collect();
    if policies
        .iter()
        .all(|p| matches!(p, ScriptPolicy::Miniscript(_)))
    {
        return conjoin_guards(policies.into_iter().filter_map(|p| match p {
            ScriptPolicy::Miniscript(clause) => Some(clause),
            _ => None,
        }))
        .into();
    }
    let mut policies: Vec<_> = policies
        .into_iter()
        .filter(|p| !is_trivial(p))
        .cloned()
        .collect();
    if policies.len() == 1 {
        policies.pop().unwrap()
    } else {
        ScriptPolicy::And(policies)
    }
}

fn is_trivial(policy: &ScriptPolicy) -> bool {
    matches!(policy, ScriptPolicy::Miniscript(Clause::Trivial))
        || matches!(policy, ScriptPolicy::And(children) if children.is_empty())
}

fn policy_as_guards(policy: ScriptPolicy) -> Vec<ScriptPolicy> {
    if is_trivial(&policy) {
        vec![]
    } else {
        vec![policy]
    }
}

fn source_policy_possible(policy: &ScriptPolicy, tx: &bitcoin::Transaction) -> bool {
    match policy {
        ScriptPolicy::Miniscript(clause) => feasibility::miniscript_policy_possible(clause, tx, 0),
        ScriptPolicy::Script(_) => true,
        // Signer lowering does not make a false covenant predicate possible.
        ScriptPolicy::Emulatable(predicate) => {
            feasibility::miniscript_policy_possible(&Clause::TxTemplate(predicate.0 .0), tx, 0)
        }
        ScriptPolicy::And(children) => children.iter().all(|p| source_policy_possible(p, tx)),
        ScriptPolicy::Or(children) => children.iter().any(|p| source_policy_possible(p, tx)),
    }
}

fn source_alternatives(policy: ScriptPolicy) -> Vec<ScriptPolicy> {
    match policy {
        ScriptPolicy::Miniscript(clause) => optimizer_flatten_policy(clause)
            .into_iter()
            .map(Into::into)
            .collect(),
        ScriptPolicy::Or(children) => children.into_iter().flat_map(source_alternatives).collect(),
        policy => vec![policy],
    }
}

fn disjoin_source(policies: BTreeSet<ScriptPolicy>) -> ScriptPolicy {
    if policies.is_empty() {
        Clause::Unsatisfiable.into()
    } else if policies.len() == 1 {
        policies.into_iter().next().unwrap()
    } else if policies
        .iter()
        .all(|p| matches!(p, ScriptPolicy::Miniscript(_)))
    {
        Clause::Thresh(miniscript::Threshold::or_n(
            policies
                .into_iter()
                .filter_map(|p| match p {
                    ScriptPolicy::Miniscript(clause) => Some(Arc::new(clause)),
                    _ => None,
                })
                .collect(),
        ))
        .into()
    } else {
        ScriptPolicy::Or(policies.into_iter().collect())
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
            let alternatives = source_alternatives(conjoin_source(existing.guards.iter()))
                .into_iter()
                .chain(source_alternatives(conjoin_source(template.guards.iter())))
                .collect();
            existing.guards = policy_as_guards(disjoin_source(alternatives));
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

enum CompiledBranch {
    Miniscript(Miniscript<XOnlyPublicKey, Tap>),
    Script(bitcoin::ScriptBuf),
}

fn compile_branches(policy: ScriptPolicy) -> Result<Vec<CompiledBranch>, CompilationError> {
    script::validate_source(&policy)?;
    match policy {
        ScriptPolicy::Miniscript(clause) => Ok(optimizer_flatten_and_compile(clause)?
            .into_iter()
            .map(CompiledBranch::Miniscript)
            .collect()),
        policy => Ok(script::lower_script_policy(&policy)?
            .into_iter()
            .map(CompiledBranch::Script)
            .collect()),
    }
}

/// Admit branches incrementally so separate actions share one output budget.
fn append_branches(
    branches: &mut Vec<CompiledBranch>,
    bytes: &mut usize,
    additions: Vec<CompiledBranch>,
) -> Result<(), CompilationError> {
    for branch in additions {
        if branches.len() == 1_024 {
            return Err(CompilationError::PolicyLimit {
                resource: "contract branches",
                limit: 1_024,
            });
        }
        let size = match &branch {
            CompiledBranch::Miniscript(script) => script.script_size(),
            CompiledBranch::Script(script) => script.len(),
        };
        *bytes = bytes
            .checked_add(size)
            .filter(|total| *total <= script::MAX_SCRIPT_BYTES)
            .ok_or(CompilationError::PolicyLimit {
                resource: "contract script bytes",
                limit: script::MAX_SCRIPT_BYTES,
            })?;
        branches.push(branch);
    }
    Ok(())
}

fn optimizer_flatten_policy(p: Clause) -> Vec<Clause> {
    match p {
        policy::Concrete::Or(v) => v
            .into_iter()
            .flat_map(|(_, b)| optimizer_flatten_policy(Arc::unwrap_or_clone(b)))
            .collect(),
        policy::Concrete::Thresh(threshold) if threshold.k() == 1 => threshold
            .into_data()
            .into_iter()
            .flat_map(|child| optimizer_flatten_policy(Arc::unwrap_or_clone(child)))
            .collect(),
        p => vec![p],
    }
}

#[cfg(test)]
mod policy_leaf_tests {
    use super::*;
    use crate::contract::actions::Guard;
    use crate::contract::Contract;
    use bitcoin::blockdata::script::Builder;
    use bitcoin::hashes::{sha256, Hash};
    use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
    use bitcoin::{Amount, Network};
    use sapio_base::policy::ScriptFragment;
    use sapio_base::timelocks::RelHeight;
    use sapio_base::LoweringPlan;

    struct PolicyContract(ScriptPolicy);

    impl PolicyContract {
        fn policy() -> Option<Guard<Self>> {
            Some(Guard::CachedPolicy(|contract| Ok(contract.0.clone()), None))
        }
    }

    impl Contract for PolicyContract {
        crate::declare! {non updatable}
        crate::declare! {finish, Self::policy}
    }

    fn key() -> Clause {
        Clause::Key(
            Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[1; 32]).unwrap())
                .x_only_public_key()
                .0,
        )
    }

    #[test]
    fn standalone_policy_leaf_matches_the_compiled_contract_script() {
        let raw = ScriptFragment::new(Builder::new().push_int(1).into_script()).unwrap();
        for policy in [
            ScriptPolicy::from(key()),
            ScriptPolicy::And(vec![
                Clause::try_from(RelHeight::from(6)).unwrap().into(),
                key().into(),
            ]),
            ScriptPolicy::And(vec![raw.into(), key().into()]),
        ] {
            let expected = compile_policy_leaf(&policy).unwrap();
            let object = PolicyContract(policy)
                .compile(Context::new(
                    Network::Regtest,
                    Amount::from_sat(1_000),
                    LoweringPlan::Native,
                    "policy_leaf".try_into().unwrap(),
                    Arc::new(Default::default()),
                    None,
                ))
                .unwrap();
            let mut input = bitcoin::psbt::Input::default();
            object
                .descriptor
                .unwrap()
                .update_psbt_input(&mut input)
                .unwrap();
            assert_eq!(input.tap_scripts.len(), 1);
            assert_eq!(input.tap_scripts.values().next().unwrap().0, expected);
        }
    }

    #[test]
    fn a_leaf_cannot_silently_discard_missing_or_alternative_branches() {
        assert!(matches!(
            compile_policy_leaf(&ScriptPolicy::Or(vec![])),
            Err(CompilationError::EmptyPolicy)
        ));
        assert!(compile_policy_leaf(&ScriptPolicy::Or(vec![key().into(), key().into()])).is_err());
    }

    #[test]
    fn a_leaf_does_not_choose_emulation_inputs() {
        let policy = Emulatable(Ctv(sha256::Hash::from_byte_array([1; 32])));
        assert!(matches!(
            compile_policy_leaf(&policy),
            Err(CompilationError::UnresolvedEmulation)
        ));
    }
}
