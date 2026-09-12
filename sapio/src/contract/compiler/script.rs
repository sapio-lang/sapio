// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Lower policy alternatives to complete Tapscript predicates.

use crate::contract::object::CovenantRequirements;
use crate::contract::CompilationError;
use bitcoin::blockdata::opcodes;
use bitcoin::{Script, ScriptBuf};
use sapio_base::covenant::LoweringPlan;
use sapio_base::miniscript::ord::Inscription;
use sapio_base::miniscript::Tap;
use sapio_base::policy::{ScriptFragment, ScriptPolicy};
use sapio_base::{Clause, EmulatedProgram};
use std::borrow::Cow;

const MAX_DEPTH: usize = 128;
const MAX_NODES: usize = 65_536;
const MAX_ALTERNATIVES: usize = 1_024;
const MAX_EXPANDED_OPERANDS: usize = 65_536;
/// Maximum source payload, expanded payload and cumulative encoded script
/// bytes for one lowering operation. This is a work limit, not a consensus
/// script-size limit or a witness-size estimate.
pub(super) const MAX_SCRIPT_BYTES: usize = 16 * 1024 * 1024;

fn limit(resource: &'static str, limit: usize) -> CompilationError {
    CompilationError::PolicyLimit { resource, limit }
}

#[derive(Clone, Copy)]
struct Expansion {
    alternatives: usize,
    operands: usize,
    payload_bytes: usize,
}

impl Expansion {
    const EMPTY: Self = Self {
        alternatives: 0,
        operands: 0,
        payload_bytes: 0,
    };
    const TRUE: Self = Self {
        alternatives: 1,
        operands: 0,
        payload_bytes: 0,
    };
    const ONE: Self = Self {
        alternatives: 1,
        operands: 1,
        payload_bytes: 0,
    };

    fn checked(
        alternatives: Option<usize>,
        operands: Option<usize>,
        payload_bytes: Option<usize>,
    ) -> Result<Self, CompilationError> {
        let alternatives = alternatives
            .filter(|count| *count <= MAX_ALTERNATIVES)
            .ok_or_else(|| limit("Taproot alternatives", MAX_ALTERNATIVES))?;
        let operands = operands
            .filter(|count| *count <= MAX_EXPANDED_OPERANDS)
            .ok_or_else(|| limit("expanded policy operands", MAX_EXPANDED_OPERANDS))?;
        let payload_bytes = payload_bytes
            .filter(|bytes| *bytes <= MAX_SCRIPT_BYTES)
            .ok_or_else(|| limit("expanded script bytes", MAX_SCRIPT_BYTES))?;
        Ok(Self {
            alternatives,
            operands,
            payload_bytes,
        })
    }

    fn or(self, other: Self) -> Result<Self, CompilationError> {
        Self::checked(
            self.alternatives.checked_add(other.alternatives),
            self.operands.checked_add(other.operands),
            self.payload_bytes.checked_add(other.payload_bytes),
        )
    }

    fn and(self, other: Self) -> Result<Self, CompilationError> {
        Self::checked(
            self.alternatives.checked_mul(other.alternatives),
            self.operands
                .checked_mul(other.alternatives)
                .and_then(|left| {
                    other
                        .operands
                        .checked_mul(self.alternatives)
                        .and_then(|right| left.checked_add(right))
                }),
            self.payload_bytes
                .checked_mul(other.alternatives)
                .and_then(|left| {
                    other
                        .payload_bytes
                        .checked_mul(self.alternatives)
                        .and_then(|right| left.checked_add(right))
                }),
        )
    }
}

#[derive(Default)]
struct Preflight<'a> {
    lowering: Option<&'a LoweringPlan>,
    nodes: usize,
    payload_bytes: usize,
}

impl Preflight<'_> {
    fn payload(&mut self, bytes: usize) -> Result<(), CompilationError> {
        self.payload_bytes = self
            .payload_bytes
            .checked_add(bytes)
            .filter(|bytes| *bytes <= MAX_SCRIPT_BYTES)
            .ok_or_else(|| limit("source script bytes", MAX_SCRIPT_BYTES))?;
        Ok(())
    }

    fn visit(&mut self, depth: usize) -> Result<(), CompilationError> {
        if depth > MAX_DEPTH {
            return Err(limit("policy depth", MAX_DEPTH));
        }
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(limit("policy nodes", MAX_NODES));
        }
        Ok(())
    }

    fn policy(
        &mut self,
        policy: &ScriptPolicy,
        depth: usize,
    ) -> Result<Expansion, CompilationError> {
        self.visit(depth)?;
        match policy {
            ScriptPolicy::Program(program) => {
                // Account for the complete instance before expansion clones it
                // into the provenance of each spending alternative.
                let payload_bytes =
                    program.instance().program().len() + program.instance().parameters().len();
                self.payload(payload_bytes)?;
                Ok(Expansion {
                    payload_bytes,
                    ..Expansion::ONE
                })
            }
            ScriptPolicy::Emulatable(_) => {
                self.visit(depth + 1)?;
                match self.lowering {
                    Some(LoweringPlan::CtvEmulation { signers, threshold })
                        if signers.len() > 1 =>
                    {
                        // Account for every generated key before derivation or
                        // cloning can expand one wrapper into a large policy.
                        for _ in signers {
                            self.visit(depth + 2)?;
                        }
                        if *threshold == 1 {
                            Expansion::checked(Some(signers.len()), Some(signers.len()), Some(0))
                        } else {
                            Ok(Expansion::ONE)
                        }
                    }
                    _ => Ok(Expansion::ONE),
                }
            }
            ScriptPolicy::Miniscript(clause) => {
                // Bound recursion before invoking the Miniscript validator.
                let expansion = self.clause(clause, depth + 1, true)?;
                super::validation::validate_policy(clause)?;
                Ok(expansion)
            }
            ScriptPolicy::Script(fragment) => {
                let payload_bytes = fragment.as_script().len();
                self.payload(payload_bytes)?;
                Ok(Expansion {
                    payload_bytes,
                    ..Expansion::ONE
                })
            }
            ScriptPolicy::And(children) => {
                let mut expansion = Expansion::TRUE;
                for child in children {
                    // Visit even when an earlier child has no alternatives:
                    // an unreachable malformed declaration is still an error.
                    expansion = expansion.and(self.policy(child, depth + 1)?)?;
                }
                Ok(expansion)
            }
            ScriptPolicy::Or(children) => {
                let mut expansion = Expansion::EMPTY;
                for child in children {
                    expansion = expansion.or(self.policy(child, depth + 1)?)?;
                }
                Ok(expansion)
            }
        }
    }

    fn clause(
        &mut self,
        clause: &Clause,
        depth: usize,
        split: bool,
    ) -> Result<Expansion, CompilationError> {
        self.visit(depth)?;
        match clause {
            Clause::Or(children) => {
                let mut expansion = Expansion::EMPTY;
                let mut payload_bytes = 0usize;
                for (_, child) in children {
                    let child = self.clause(child, depth + 1, split)?;
                    if split {
                        expansion = expansion.or(child)?;
                    } else {
                        payload_bytes = payload_bytes
                            .checked_add(child.payload_bytes)
                            .ok_or_else(|| limit("expanded script bytes", MAX_SCRIPT_BYTES))?;
                    }
                }
                Ok(if split {
                    expansion
                } else {
                    Expansion {
                        payload_bytes,
                        ..Expansion::ONE
                    }
                })
            }
            Clause::Thresh(threshold) if split && threshold.k() == 1 => {
                let mut expansion = Expansion::EMPTY;
                for child in threshold.iter() {
                    expansion = expansion.or(self.clause(child, depth + 1, true)?)?;
                }
                Ok(expansion)
            }
            Clause::And(children) => {
                let mut payload_bytes = 0usize;
                for child in children {
                    payload_bytes = payload_bytes
                        .checked_add(self.clause(child, depth + 1, false)?.payload_bytes)
                        .ok_or_else(|| limit("expanded script bytes", MAX_SCRIPT_BYTES))?;
                }
                Ok(Expansion {
                    payload_bytes,
                    ..Expansion::ONE
                })
            }
            Clause::Thresh(threshold) => {
                let mut payload_bytes = 0usize;
                for child in threshold.iter() {
                    payload_bytes = payload_bytes
                        .checked_add(self.clause(child, depth + 1, false)?.payload_bytes)
                        .ok_or_else(|| limit("expanded script bytes", MAX_SCRIPT_BYTES))?;
                }
                Ok(Expansion {
                    payload_bytes,
                    ..Expansion::ONE
                })
            }
            Clause::Inscribe(inscription, child) => {
                let payload_bytes = inscription_payload_bytes(inscription)?;
                self.payload(payload_bytes)?;
                let payload_bytes = payload_bytes
                    .checked_add(self.clause(child, depth + 1, false)?.payload_bytes)
                    .ok_or_else(|| limit("expanded script bytes", MAX_SCRIPT_BYTES))?;
                Ok(Expansion {
                    payload_bytes,
                    ..Expansion::ONE
                })
            }
            _ => Ok(Expansion::ONE),
        }
    }
}

fn inscription_payload_bytes(inscription: &Inscription) -> Result<usize, CompilationError> {
    // Inspect the owned payload without calling size_guess(), which encodes an
    // entire envelope. Body and metadata legitimately span many 520-byte pushes.
    [
        &inscription.body,
        &inscription.content_encoding,
        &inscription.content_type,
        &inscription.delegate,
        &inscription.metadata,
        &inscription.metaprotocol,
        &inscription.parent,
        &inscription.pointer,
    ]
    .into_iter()
    .filter_map(|field| field.as_ref())
    .try_fold(0usize, |bytes, field| {
        bytes
            .checked_add(field.len())
            .ok_or_else(|| limit("source script bytes", MAX_SCRIPT_BYTES))
    })
}

#[derive(Clone, Copy)]
enum Operand<'a> {
    Miniscript(&'a Clause),
    Program(&'a EmulatedProgram),
    Script(&'a ScriptFragment),
}

/// Cumulative provenance allocation across all actions in one contract.
#[derive(Default)]
pub(super) struct RecordedPolicyBudget(Preflight<'static>);

impl RecordedPolicyBudget {
    fn admit(&mut self, alternatives: &[Vec<Operand<'_>>]) -> Result<(), CompilationError> {
        for operands in alternatives {
            if !operands
                .iter()
                .any(|operand| matches!(operand, Operand::Program(_)))
            {
                continue;
            }
            let depth = usize::from(operands.len() > 1);
            if depth != 0 {
                self.0.visit(0)?;
            }
            for operand in operands {
                self.0.visit(depth)?;
                match operand {
                    Operand::Miniscript(clause) => {
                        // This complete native subtree is cloned into the
                        // record, even when its choices remain inside a leaf.
                        self.0.clause(clause, depth + 1, false)?;
                    }
                    Operand::Program(program) => {
                        self.0.payload(
                            program.instance().program().len()
                                + program.instance().parameters().len(),
                        )?;
                    }
                    Operand::Script(fragment) => self.0.payload(fragment.as_script().len())?,
                }
            }
        }
        Ok(())
    }
}

fn expand_clause(clause: &Clause) -> Vec<Vec<Operand<'_>>> {
    match clause {
        Clause::Or(children) => children
            .iter()
            .flat_map(|(_, child)| expand_clause(child))
            .collect(),
        Clause::Thresh(threshold) if threshold.k() == 1 => threshold
            .iter()
            .flat_map(|child| expand_clause(child))
            .collect(),
        clause => vec![vec![Operand::Miniscript(clause)]],
    }
}

fn expand(policy: &ScriptPolicy) -> Vec<Vec<Operand<'_>>> {
    expand_inner(policy, false, false)
}

fn expand_inner(
    policy: &ScriptPolicy,
    retain_native_choices: bool,
    conjoined: bool,
) -> Vec<Vec<Operand<'_>>> {
    match policy {
        ScriptPolicy::Miniscript(clause) if retain_native_choices && conjoined => {
            vec![vec![Operand::Miniscript(clause)]]
        }
        ScriptPolicy::Miniscript(clause) => expand_clause(clause),
        ScriptPolicy::Program(program) => vec![vec![Operand::Program(program)]],
        ScriptPolicy::Emulatable(_) => unreachable!("resolved before expansion"),
        ScriptPolicy::Script(fragment) => vec![vec![Operand::Script(fragment)]],
        ScriptPolicy::Or(children) => children
            .iter()
            .flat_map(|child| expand_inner(child, retain_native_choices, conjoined))
            .collect(),
        ScriptPolicy::And(children) => {
            let mut alternatives = vec![vec![]];
            for child in children {
                if alternatives.is_empty() {
                    // Preflight already validated every unreachable child.
                    break;
                }
                let next = expand_inner(child, retain_native_choices, true);
                if let [right] = next.as_slice() {
                    // A long conjunction of single predicates must append in
                    // place, not repeatedly copy its growing prefix.
                    for left in &mut alternatives {
                        left.extend_from_slice(right);
                    }
                    continue;
                }
                alternatives = alternatives
                    .into_iter()
                    .flat_map(|left| {
                        next.iter().map(move |right| {
                            let mut operands = Vec::with_capacity(left.len() + right.len());
                            operands.extend_from_slice(&left);
                            operands.extend_from_slice(right);
                            operands
                        })
                    })
                    .collect();
            }
            alternatives
        }
    }
}

fn encoded_size(
    total: usize,
    script_size: usize,
    separator: bool,
) -> Result<usize, CompilationError> {
    total
        .checked_add(script_size)
        .and_then(|bytes| bytes.checked_add(usize::from(separator)))
        .filter(|bytes| *bytes <= MAX_SCRIPT_BYTES)
        .ok_or_else(|| limit("encoded script bytes", MAX_SCRIPT_BYTES))
}

fn compile_run(
    clauses: &[Cow<'_, Clause>],
    encoded_bytes: usize,
    separator: bool,
) -> Result<ScriptBuf, CompilationError> {
    // Combining the complete run lets a key/CTV predicate protect an adjacent
    // timelock or hashlock. Do not reorder predicates across an opaque script.
    // The current public Miniscript compiler requires a safe run; an opaque
    // neighbor cannot establish that proof. A custom backend can instead emit
    // the complete predicate, including its timelock or hashlock, as raw script.
    let clause = super::conjoin_guards(clauses.iter().map(|clause| clause.as_ref()));
    let compiled = clause.compile::<Tap>()?;
    // Check the encoded size before materializing the native script as well.
    encoded_size(encoded_bytes, compiled.script_size(), separator)?;
    Ok(compiled.encode())
}

fn append_script(
    bytes: &mut Vec<u8>,
    script: &Script,
    has_predicate: &mut bool,
    encoded_bytes: &mut usize,
) -> Result<(), CompilationError> {
    let next_size = encoded_size(*encoded_bytes, script.len(), *has_predicate)?;
    if *has_predicate {
        bytes.push(opcodes::all::OP_VERIFY.to_u8());
    }
    bytes.extend_from_slice(script.as_bytes());
    *has_predicate = true;
    *encoded_bytes = next_size;
    Ok(())
}

fn compile_alternative(
    operands: &[Operand<'_>],
    encoded_bytes: &mut usize,
) -> Result<ScriptBuf, CompilationError> {
    if operands.is_empty() {
        // The IR's empty conjunction is explicitly true. It has no Miniscript
        // clause whose standalone signature-safety rules need to be applied.
        *encoded_bytes = encoded_size(*encoded_bytes, 1, false)?;
        return Ok(ScriptBuf::from(vec![opcodes::OP_TRUE.to_u8()]));
    }
    let mut bytes = vec![];
    let mut has_predicate = false;
    let mut clauses = vec![];
    for operand in operands {
        match operand {
            Operand::Miniscript(clause) => clauses.push(Cow::Borrowed(*clause)),
            Operand::Program(program) => clauses.push(Cow::Owned(program_clause(program)?)),
            Operand::Script(script) => {
                if !clauses.is_empty() {
                    let script = compile_run(&clauses, *encoded_bytes, has_predicate)?;
                    append_script(&mut bytes, &script, &mut has_predicate, encoded_bytes)?;
                    clauses.clear();
                }
                // Borrow the source fragment directly. A raw script shared by
                // many alternatives must not first be cloned into temporary runs.
                append_script(
                    &mut bytes,
                    script.as_script(),
                    &mut has_predicate,
                    encoded_bytes,
                )?;
            }
        }
    }
    if !clauses.is_empty() {
        let script = compile_run(&clauses, *encoded_bytes, has_predicate)?;
        append_script(&mut bytes, &script, &mut has_predicate, encoded_bytes)?;
    }
    Ok(ScriptBuf::from(bytes))
}

fn program_clause(program: &EmulatedProgram) -> Result<Clause, CompilationError> {
    program
        .derive_public_key()
        .map(Clause::Key)
        .map_err(|error| CompilationError::Custom(error.to_string().into()))
}

pub(super) fn contains_program(policy: &ScriptPolicy) -> bool {
    match policy {
        ScriptPolicy::Program(_) => true,
        ScriptPolicy::And(children) | ScriptPolicy::Or(children) => {
            children.iter().any(contains_program)
        }
        _ => false,
    }
}

fn contains_raw(policy: &ScriptPolicy) -> bool {
    match policy {
        ScriptPolicy::Script(_) => true,
        ScriptPolicy::And(children) | ScriptPolicy::Or(children) => {
            children.iter().any(contains_raw)
        }
        _ => false,
    }
}

/// Keep the exact source of each expanded program-bearing alternative. The
/// payload budget is shared by every action contributing to this contract.
pub(super) fn lower_program_branches(
    policy: &ScriptPolicy,
    budget: &mut RecordedPolicyBudget,
) -> Result<Vec<super::CompiledBranch>, CompilationError> {
    Preflight::default().policy(policy, 0)?;
    reject_unresolved(policy)?;
    let alternatives = expand_inner(policy, !contains_raw(policy), false);
    budget.admit(&alternatives)?;
    let mut encoded_bytes = 0;
    alternatives
        .iter()
        .map(|operands| {
            let has_program = operands
                .iter()
                .any(|operand| matches!(operand, Operand::Program(_)));
            let native = !operands.is_empty()
                && operands
                    .iter()
                    .all(|operand| !matches!(operand, Operand::Script(_)));
            let (script, retained_program) = if native {
                let clauses = operands
                    .iter()
                    .map(|operand| match operand {
                        Operand::Miniscript(clause) => Ok(Cow::Borrowed(*clause)),
                        Operand::Program(program) => program_clause(program).map(Cow::Owned),
                        Operand::Script(_) => unreachable!("native operands only"),
                    })
                    .collect::<Result<Vec<_>, CompilationError>>()?;
                let clause = super::conjoin_guards(clauses.iter().map(|clause| clause.as_ref()));
                let compiled = clause.compile::<Tap>()?;
                encoded_bytes = encoded_size(encoded_bytes, compiled.script_size(), false)?;
                // A false conjunction may have erased its signature operands.
                // Reuse the derived clauses rather than deriving keys again.
                let retained = operands.iter().zip(&clauses).any(|(operand, clause)| {
                    matches!(operand, Operand::Program(_))
                        && matches!(clause.as_ref(),
                        Clause::Key(key) if compiled.iter_pk().any(|candidate| candidate == *key))
                });
                (super::CompiledScript::Miniscript(compiled), retained)
            } else {
                (
                    super::CompiledScript::Script(compile_alternative(
                        operands,
                        &mut encoded_bytes,
                    )?),
                    has_program,
                )
            };
            let program_policy = retained_program.then(|| {
                let mut source: Vec<_> = operands
                    .iter()
                    .map(|operand| match operand {
                        Operand::Miniscript(clause) => ScriptPolicy::Miniscript((*clause).clone()),
                        Operand::Program(program) => ScriptPolicy::Program((*program).clone()),
                        Operand::Script(fragment) => ScriptPolicy::Script((*fragment).clone()),
                    })
                    .collect();
                if source.len() == 1 {
                    source.pop().unwrap()
                } else {
                    ScriptPolicy::And(source)
                }
            });
            Ok(super::CompiledBranch {
                script,
                program_policy,
            })
        })
        .collect()
}

fn reject_unresolved(policy: &ScriptPolicy) -> Result<(), CompilationError> {
    let mut pending = vec![policy];
    while let Some(policy) = pending.pop() {
        match policy {
            ScriptPolicy::Emulatable(_) => return Err(CompilationError::UnresolvedEmulation),
            ScriptPolicy::And(children) | ScriptPolicy::Or(children) => pending.extend(children),
            _ => {}
        }
    }
    Ok(())
}

/// Compile disjunctive alternatives after validating the complete source and
/// bounding its expansion. Each raw fragment must supply one complete Boolean
/// predicate; the caller's backend owns its witness and authorization semantics.
/// Every adjacent Miniscript run retains Miniscript's compilation checks.
/// Source raw/inscription payloads, their expanded repetitions and the total
/// encoded output are each limited to 16 MiB per invocation. This bounds
/// lowering work; it is not a consensus limit or a witness-size guarantee.
pub(crate) fn lower_script_policy(
    policy: &ScriptPolicy,
) -> Result<Vec<ScriptBuf>, CompilationError> {
    validate_source(policy)?;
    if contains_program(policy) {
        return lower_program_branches(policy, &mut RecordedPolicyBudget::default()).map(
            |branches| {
                branches
                    .into_iter()
                    .map(|branch| branch.script.into_script())
                    .collect()
            },
        );
    }
    reject_unresolved(policy)?;
    let mut encoded_bytes = 0;
    expand(policy)
        .iter()
        .map(|alternative| compile_alternative(alternative, &mut encoded_bytes))
        .collect()
}

/// Resolve only explicitly wrapped predicates, retaining their public inputs.
/// Validate source and generated node counts before any derivation or copying.
pub(super) fn resolve_emulation(
    policy: &ScriptPolicy,
    requirements: &mut CovenantRequirements,
) -> Result<ScriptPolicy, CompilationError> {
    requirements.lowering.validate()?;
    Preflight {
        lowering: Some(&requirements.lowering),
        ..Preflight::default()
    }
    .policy(policy, 0)?;
    fn resolve(
        policy: &ScriptPolicy,
        requirements: &mut CovenantRequirements,
    ) -> Result<(ScriptPolicy, bool), CompilationError> {
        Ok(match policy {
            ScriptPolicy::Emulatable(predicate) => {
                requirements.predicates.insert(predicate.0);
                (requirements.lowering.lower_ctv(predicate.0)?.into(), true)
            }
            ScriptPolicy::And(children) | ScriptPolicy::Or(children) => {
                let mut changed = false;
                let children = children
                    .iter()
                    .map(|child| {
                        let (child, resolved) = resolve(child, requirements)?;
                        changed |= resolved;
                        Ok(child)
                    })
                    .collect::<Result<Vec<_>, CompilationError>>()?;
                let policy = if matches!(policy, ScriptPolicy::And(_)) {
                    if changed {
                        // The automatic wrapped CTV can still share a native
                        // Miniscript run with its surrounding guards.
                        super::conjoin_source(children.iter())
                    } else {
                        ScriptPolicy::And(children)
                    }
                } else {
                    ScriptPolicy::Or(children)
                };
                (policy, changed)
            }
            policy => (policy.clone(), false),
        })
    }
    let (resolved, _) = resolve(policy, requirements)?;
    validate_source(&resolved)?;
    Ok(resolved)
}

/// Validate and bound source policies before cloning, simplification or
/// enumeration can erase malformed declarations or expand untrusted input.
pub(crate) fn validate_source(policy: &ScriptPolicy) -> Result<(), CompilationError> {
    Preflight::default().policy(policy, 0).map(|_| ())
}

pub(super) fn validate_sources<'a>(
    policies: impl IntoIterator<Item = &'a ScriptPolicy>,
) -> Result<(), CompilationError> {
    let mut preflight = Preflight::default();
    let mut expansion = Expansion::EMPTY;
    for policy in policies {
        expansion = expansion.or(preflight.policy(policy, 0)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::bip32::{ChainCode, Xpriv, Xpub};
    use bitcoin::blockdata::script::Builder;
    use bitcoin::hashes::{sha256, Hash};
    use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
    use bitcoin::Network;
    use sapio_base::covenant::{Ctv, Emulatable};
    use sapio_base::miniscript::ord::envelope::Envelope;
    use sapio_base::miniscript::policy::compiler::CompilerError;
    use sapio_base::policy::ScriptFragment;
    use std::sync::Arc;

    fn public_lowering(signer_count: usize, threshold: u8) -> LoweringPlan {
        let secret = Xpriv::new_master(Network::Testnet, &[17; 32]).unwrap();
        let root = Xpub::from_priv(&Secp256k1::new(), &secret);
        LoweringPlan::CtvEmulation {
            signers: (0..signer_count)
                .map(|index| {
                    let mut signer = root;
                    let mut chain_code = [0; 32];
                    chain_code[..8].copy_from_slice(&(index as u64).to_be_bytes());
                    signer.chain_code = ChainCode::from(chain_code);
                    signer
                })
                .collect(),
            threshold,
        }
    }

    fn wrapped(byte: u8) -> ScriptPolicy {
        Emulatable(Ctv(sha256::Hash::from_byte_array([byte; 32]))).into()
    }

    #[test]
    fn generated_signer_nodes_are_bounded_before_any_wrapper_is_resolved() {
        let mut requirements = CovenantRequirements {
            lowering: public_lowering(8, 2),
            predicates: Default::default(),
        };
        // Each wrapper is small in the source but generates a threshold and
        // eight key leaves. Check both sides of that generated-node boundary.
        let at_limit = ScriptPolicy::And(vec![wrapped(1); 6_553]);
        Preflight {
            lowering: Some(&requirements.lowering),
            ..Preflight::default()
        }
        .policy(&at_limit, 0)
        .unwrap();
        let over_limit = ScriptPolicy::And(vec![wrapped(1); 6_554]);
        validate_source(&over_limit).unwrap();
        assert!(matches!(
            resolve_emulation(&over_limit, &mut requirements),
            Err(CompilationError::PolicyLimit {
                resource: "policy nodes",
                limit: MAX_NODES,
            })
        ));
        // Resolution records a wrapper before deriving its keys. An empty set
        // therefore also establishes rejection before that work started.
        assert!(requirements.predicates.is_empty());
    }

    #[test]
    fn signer_choices_respect_leaf_and_cartesian_alternative_limits() {
        let mut too_many_signers = CovenantRequirements {
            lowering: public_lowering(MAX_ALTERNATIVES + 1, 1),
            predicates: Default::default(),
        };
        assert!(matches!(
            resolve_emulation(&wrapped(1), &mut too_many_signers),
            Err(CompilationError::PolicyLimit {
                resource: "Taproot alternatives",
                limit: MAX_ALTERNATIVES,
            })
        ));
        assert!(too_many_signers.predicates.is_empty());

        let policy = |count: u8| {
            ScriptPolicy::And(
                (0..count)
                    // Raw separators retain independently selectable runs.
                    .flat_map(|index| [wrapped(index), raw(i64::from(index))])
                    .collect(),
            )
        };
        let mut requirements = CovenantRequirements {
            lowering: public_lowering(2, 1),
            predicates: Default::default(),
        };
        let resolved = resolve_emulation(&policy(10), &mut requirements).unwrap();
        let expansion = Preflight::default().policy(&resolved, 0).unwrap();
        assert_eq!(expansion.alternatives, MAX_ALTERNATIVES);
        assert_eq!(requirements.predicates.len(), 10);
        let before = requirements.clone();
        assert!(matches!(
            resolve_emulation(&policy(11), &mut requirements),
            Err(CompilationError::PolicyLimit {
                resource: "Taproot alternatives",
                limit: MAX_ALTERNATIVES,
            })
        ));
        assert_eq!(requirements, before);
    }

    #[test]
    fn generated_threshold_depth_is_checked_before_key_derivation() {
        let mut requirements = CovenantRequirements {
            lowering: public_lowering(2, 2),
            predicates: Default::default(),
        };
        let mut policy = wrapped(1);
        for _ in 0..MAX_DEPTH - 2 {
            policy = ScriptPolicy::Or(vec![policy]);
        }
        Preflight {
            lowering: Some(&requirements.lowering),
            ..Preflight::default()
        }
        .policy(&policy, 0)
        .unwrap();
        policy = ScriptPolicy::Or(vec![policy]);
        validate_source(&policy).unwrap();
        assert!(matches!(
            resolve_emulation(&policy, &mut requirements),
            Err(CompilationError::PolicyLimit {
                resource: "policy depth",
                limit: MAX_DEPTH,
            })
        ));
        assert!(requirements.predicates.is_empty());
    }

    #[test]
    fn resolving_wrappers_preserves_native_predicates_and_raw_operand_order() {
        let native = ScriptPolicy::from(Clause::TxTemplate(sha256::Hash::from_byte_array([2; 32])));
        let original = ScriptPolicy::Or(vec![
            wrapped(1),
            ScriptPolicy::And(vec![raw(4), native.clone(), wrapped(3), raw(5)]),
        ]);
        for lowering in [LoweringPlan::Native, public_lowering(2, 2)] {
            let mut requirements = CovenantRequirements {
                lowering: lowering.clone(),
                predicates: Default::default(),
            };
            let first = Ctv(sha256::Hash::from_byte_array([1; 32]));
            let second = Ctv(sha256::Hash::from_byte_array([3; 32]));
            let expected = ScriptPolicy::Or(vec![
                lowering.lower_ctv(first).unwrap().into(),
                ScriptPolicy::And(vec![
                    raw(4),
                    native.clone(),
                    lowering.lower_ctv(second).unwrap().into(),
                    raw(5),
                ]),
            ]);
            assert_eq!(
                resolve_emulation(&original, &mut requirements).unwrap(),
                expected
            );
            assert_eq!(
                requirements.predicates,
                [first, second].into_iter().collect()
            );
            assert_eq!(requirements.lowering, lowering);
        }
    }

    fn raw(value: i64) -> ScriptPolicy {
        ScriptPolicy::Script(
            ScriptFragment::new(
                Builder::new()
                    .push_int(value)
                    .push_opcode(opcodes::all::OP_DROP)
                    .push_int(1)
                    .into_script(),
            )
            .unwrap(),
        )
    }

    fn bytes(policy: &ScriptPolicy) -> Vec<u8> {
        match policy {
            ScriptPolicy::Script(fragment) => fragment.as_script().as_bytes().to_vec(),
            _ => unreachable!(),
        }
    }

    fn raw_bytes(size: usize) -> ScriptPolicy {
        let mut bytes = vec![opcodes::all::OP_NOP.to_u8(); size];
        if let Some(last) = bytes.last_mut() {
            *last = opcodes::OP_TRUE.to_u8();
        }
        ScriptPolicy::Script(ScriptFragment::new(ScriptBuf::from(bytes)).unwrap())
    }

    #[test]
    fn raw_byte_repetition_is_rejected_before_cartesian_materialization() {
        let fragment = raw_bytes(MAX_SCRIPT_BYTES / MAX_ALTERNATIVES + 1);
        let choice = ScriptPolicy::Or(vec![ScriptPolicy::And(vec![]); 2]);
        for raw_first in [false, true] {
            let mut children = vec![choice.clone(); 10];
            if raw_first {
                children.insert(0, fragment.clone());
            } else {
                children.push(fragment.clone());
            }
            let policy = ScriptPolicy::And(children);
            // The source owns only about 16 KiB of script data, but expansion
            // would repeat it in 1,024 alternatives and exceed the byte budget.
            assert!(matches!(
                validate_source(&policy),
                Err(CompilationError::PolicyLimit {
                    resource: "expanded script bytes",
                    limit: MAX_SCRIPT_BYTES,
                })
            ));
            assert!(matches!(
                lower_script_policy(&policy),
                Err(CompilationError::PolicyLimit {
                    resource: "expanded script bytes",
                    limit: MAX_SCRIPT_BYTES,
                })
            ));
        }
    }

    #[test]
    fn cumulative_encoding_includes_generated_verification_opcodes() {
        let policy = ScriptPolicy::And(vec![
            raw_bytes(MAX_SCRIPT_BYTES / MAX_ALTERNATIVES),
            ScriptPolicy::Or(vec![ScriptPolicy::And(vec![]); MAX_ALTERNATIVES]),
            raw_bytes(0),
        ]);
        // The repeated raw payload fits exactly. Each branch also inserts a
        // VERIFY between its two fragments, so the complete output does not.
        validate_source(&policy).unwrap();
        assert!(matches!(
            lower_script_policy(&policy),
            Err(CompilationError::PolicyLimit {
                resource: "encoded script bytes",
                limit: MAX_SCRIPT_BYTES,
            })
        ));
    }

    #[test]
    fn source_payloads_are_bounded_even_in_unreachable_branches() {
        let policy = ScriptPolicy::And(vec![
            ScriptPolicy::Or(vec![]),
            raw_bytes(MAX_SCRIPT_BYTES + 1),
        ]);
        assert!(matches!(
            validate_source(&policy),
            Err(CompilationError::PolicyLimit {
                resource: "source script bytes",
                limit: MAX_SCRIPT_BYTES,
            })
        ));
    }

    #[test]
    fn byte_budget_allows_large_raw_leaves_and_chunked_inscription_bodies() {
        let raw = lower_script_policy(&raw_bytes(65_537)).unwrap();
        assert_eq!(raw[0].len(), 65_537);

        let key =
            Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[3; 32]).unwrap())
                .x_only_public_key()
                .0;
        let inscription = Inscription::new(Some(b"text/plain".to_vec()), Some(vec![42; 1_025]));
        let policy = ScriptPolicy::Miniscript(Clause::Inscribe(
            Box::new(inscription.clone()),
            Arc::new(Clause::Key(key)),
        ));
        let scripts = lower_script_policy(&policy).unwrap();
        let envelopes = Envelope::from_tapscript(&scripts[0], 0).unwrap();
        assert_eq!(envelopes.len(), 1);
        let parsed: Envelope<Inscription> = envelopes.into_iter().next().unwrap().into();
        assert_eq!(parsed.payload, inscription);
    }

    #[test]
    fn nested_alternatives_expand_in_declared_operand_order() {
        let left: Vec<_> = (0..5).map(raw).collect();
        let right: Vec<_> = (10..14).map(raw).collect();
        let policy = ScriptPolicy::And(vec![
            ScriptPolicy::Or(left.clone()),
            ScriptPolicy::Or(vec![
                ScriptPolicy::Or(right[..2].to_vec()),
                ScriptPolicy::Or(right[2..].to_vec()),
            ]),
        ]);
        let scripts = lower_script_policy(&policy).unwrap();
        assert_eq!(scripts.len(), 20);
        for (index, script) in scripts.iter().enumerate() {
            let mut expected = bytes(&left[index / 4]);
            expected.push(opcodes::all::OP_VERIFY.to_u8());
            expected.extend_from_slice(&bytes(&right[index % 4]));
            assert_eq!(script.as_bytes(), expected);
        }
    }

    #[test]
    fn empty_connectives_have_boolean_identity_semantics() {
        assert!(lower_script_policy(&ScriptPolicy::Or(vec![]))
            .unwrap()
            .is_empty());
        assert_eq!(
            lower_script_policy(&ScriptPolicy::And(vec![])).unwrap(),
            vec![Builder::new().push_int(1).into_script()]
        );
        assert!(
            lower_script_policy(&ScriptPolicy::And(vec![ScriptPolicy::Or(vec![]), raw(1),]))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn unreachable_miniscript_still_requires_structural_validation() {
        let policy = ScriptPolicy::And(vec![
            ScriptPolicy::Or(vec![]),
            ScriptPolicy::Miniscript(Clause::And(vec![])),
        ]);
        assert!(matches!(
            lower_script_policy(&policy),
            Err(CompilationError::Miniscript(_))
        ));
    }

    #[test]
    fn zero_relative_policy_lock_is_rejected_before_simplification() {
        use sapio_base::miniscript::RelLockTime;
        use sapio_base::timelocks::LockTimeError;

        for zero in [RelLockTime::ZERO, RelLockTime::from_height(0)] {
            let zero = ScriptPolicy::Miniscript(Clause::Older(zero));
            for policy in [
                zero.clone(),
                ScriptPolicy::And(vec![ScriptPolicy::Or(vec![]), zero]),
            ] {
                assert!(matches!(
                    lower_script_policy(&policy),
                    Err(CompilationError::TimeLockError(
                        LockTimeError::InvalidPolicyLockTime(0)
                    ))
                ));
            }
        }
    }

    #[test]
    fn pure_miniscript_runs_preserve_combined_safety_and_optimization() {
        let key =
            Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[3; 32]).unwrap())
                .x_only_public_key()
                .0;
        let key = Clause::Key(key);
        let time = Clause::Older(sapio_base::miniscript::RelLockTime::from_height(16));
        let policy = ScriptPolicy::And(vec![
            ScriptPolicy::Miniscript(key.clone()),
            ScriptPolicy::Miniscript(time.clone()),
        ]);
        assert_eq!(
            lower_script_policy(&policy).unwrap(),
            vec![
                Clause::And(vec![std::sync::Arc::new(key), std::sync::Arc::new(time)])
                    .compile::<Tap>()
                    .unwrap()
                    .encode()
            ]
        );
    }

    #[test]
    fn opaque_neighbors_do_not_silently_bypass_miniscript_safety_checks() {
        let key =
            Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[3; 32]).unwrap())
                .x_only_public_key()
                .0;
        let raw_key = ScriptPolicy::Script(
            ScriptFragment::new(
                Builder::new()
                    .push_slice(&key.serialize())
                    .push_opcode(opcodes::all::OP_CHECKSIG)
                    .into_script(),
            )
            .unwrap(),
        );
        let time = ScriptPolicy::Miniscript(Clause::Older(
            sapio_base::miniscript::RelLockTime::from_height(16),
        ));
        for children in [vec![raw_key.clone(), time.clone()], vec![time, raw_key]] {
            assert!(matches!(
                lower_script_policy(&ScriptPolicy::And(children)),
                Err(CompilationError::Miniscript(CompilerError::TopLevelNonSafe))
            ));
        }
    }

    #[test]
    fn cartesian_expansion_is_bounded_before_materialization() {
        let choice = ScriptPolicy::Or(vec![raw(1), raw(2)]);
        let at_limit = ScriptPolicy::And(vec![choice.clone(); 10]);
        assert_eq!(lower_script_policy(&at_limit).unwrap().len(), 1_024);
        let too_many = ScriptPolicy::And(vec![choice; 11]);
        assert!(matches!(
            lower_script_policy(&too_many),
            Err(CompilationError::PolicyLimit {
                resource: "Taproot alternatives",
                limit: 1_024
            })
        ));
        let mut large_product = vec![ScriptPolicy::Or(vec![raw(1); 1_024])];
        large_product.extend(vec![raw(2); 64]);
        assert!(matches!(
            lower_script_policy(&ScriptPolicy::And(large_product)),
            Err(CompilationError::PolicyLimit {
                resource: "expanded policy operands",
                limit: 65_536
            })
        ));
    }

    #[test]
    fn source_depth_and_nodes_are_checked_before_recursive_compilation() {
        let mut deep = raw(1);
        for _ in 0..=MAX_DEPTH {
            deep = ScriptPolicy::And(vec![deep]);
        }
        assert!(matches!(
            lower_script_policy(&deep),
            Err(CompilationError::PolicyLimit {
                resource: "policy depth",
                limit: 128
            })
        ));
        assert!(matches!(
            lower_script_policy(&ScriptPolicy::And(vec![raw(1); MAX_NODES])),
            Err(CompilationError::PolicyLimit {
                resource: "policy nodes",
                limit: 65_536
            })
        ));
    }

    fn program(payload: usize) -> ScriptPolicy {
        let root = Xpub::from_priv(
            &Secp256k1::new(),
            &Xpriv::new_master(Network::Regtest, &[53; 32]).unwrap(),
        );
        EmulatedProgram::new(
            sapio_base::program::ProgramInstance::wasm_v2(vec![0; payload], vec![]).unwrap(),
            root,
        )
        .unwrap()
        .into()
    }

    #[test]
    fn program_payload_expansion_is_bounded_before_source_is_copied() {
        let source = ScriptPolicy::And(vec![
            program(sapio_base::program::MAX_PROGRAM_BYTES),
            ScriptPolicy::Or(vec![ScriptPolicy::And(vec![]); 257]),
        ]);
        assert!(matches!(
            lower_script_policy(&source),
            Err(CompilationError::PolicyLimit {
                resource: "expanded script bytes",
                limit: MAX_SCRIPT_BYTES
            })
        ));
    }

    #[test]
    fn every_actions_program_records_share_one_payload_budget() {
        let source = ScriptPolicy::And(vec![
            program(sapio_base::program::MAX_PROGRAM_BYTES),
            ScriptPolicy::Or(vec![ScriptPolicy::And(vec![]); 128]),
        ]);
        let mut budget = RecordedPolicyBudget::default();
        assert_eq!(
            lower_program_branches(&source, &mut budget).unwrap().len(),
            128
        );
        assert_eq!(
            lower_program_branches(&source, &mut budget).unwrap().len(),
            128
        );
        assert!(matches!(
            lower_program_branches(&source, &mut budget),
            Err(CompilationError::PolicyLimit {
                resource: "source script bytes",
                limit: MAX_SCRIPT_BYTES
            })
        ));
    }

    #[test]
    fn expanded_native_subtrees_are_bounded_before_record_cloning() {
        let key =
            Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[1; 32]).unwrap())
                .x_only_public_key()
                .0;
        let native = Clause::Thresh(sapio_base::miniscript::Threshold::and_n(
            vec![std::sync::Arc::new(Clause::Key(key)); 1_024],
        ));
        let source = ScriptPolicy::And(vec![
            program(0),
            native.into(),
            ScriptPolicy::Or(vec![ScriptPolicy::And(vec![]); 64]),
        ]);
        // The original source and reference expansion fit; copied native
        // subtrees would exceed the aggregate policy-node budget.
        validate_source(&source).unwrap();
        assert!(matches!(
            lower_script_policy(&source),
            Err(CompilationError::PolicyLimit {
                resource: "policy nodes",
                limit: MAX_NODES
            })
        ));
    }

    #[test]
    fn unreachable_program_branches_do_not_hide_invalid_native_source() {
        let source = ScriptPolicy::And(vec![
            ScriptPolicy::Or(vec![]),
            program(0),
            Clause::And(vec![]).into(),
        ]);
        assert!(matches!(
            lower_script_policy(&source),
            Err(CompilationError::Miniscript(CompilerError::NonBinaryArgAnd))
        ));
    }
}
