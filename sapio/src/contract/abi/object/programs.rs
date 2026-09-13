//! Compiler-retained program policies and their explicit signing locations.

use super::{ArtifactError, ArtifactErrorKind, Object};
use crate::contract::compiler::{validate_policy_sources, validate_program_policy};
use bitcoin::blockdata::opcodes::all::OP_CHECKSIG;
use bitcoin::blockdata::script::Builder;
use bitcoin::psbt::Input;
use bitcoin::taproot::{LeafVersion, TapLeafHash};
use bitcoin::XOnlyPublicKey;
use sapio_base::miniscript::{Miniscript, Tap};
use sapio_base::policy::ScriptPolicy;
use sapio_base::program::{EmulatedProgram, ProgramSpendPath};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// One complete program-bearing branch source and its final spending locations.
///
/// Optional SIMP annotations are separate. These records describe available
/// signature slots, not an instruction to authorize every alternative. Their
/// validation establishes internal consistency, not the origin of an artifact.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProgramPolicy {
    /// Complete branch source, retaining native guards and exact program identities.
    pub policy: ScriptPolicy,
    /// Locations remaining after the compiler selects the internal key.
    pub paths: BTreeSet<ProgramSpendPath>,
}

/// An explicitly selectable program signature slot derived from a valid artifact.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProgramRequirement {
    /// Exact evaluator, program, parameters and public oracle root.
    pub program: EmulatedProgram,
    /// The one location the caller elects to authorize.
    pub path: ProgramSpendPath,
}

fn programs(policy: &ScriptPolicy) -> BTreeSet<&EmulatedProgram> {
    let mut pending = vec![policy];
    let mut programs = BTreeSet::new();
    while let Some(policy) = pending.pop() {
        match policy {
            ScriptPolicy::Program(program) => {
                programs.insert(program);
            }
            ScriptPolicy::And(children) | ScriptPolicy::Or(children) => pending.extend(children),
            _ => {}
        }
    }
    programs
}

impl Object {
    /// Enumerate this output's available program slots after validating the graph.
    ///
    /// The caller selects one slot and provides its evidence explicitly. Removing
    /// or adding optional metadata cannot change this result. This cannot recover
    /// source that an artifact producer replaced with ordinary public-key policy.
    pub fn program_requirements(&self) -> Result<BTreeSet<ProgramRequirement>, ArtifactError> {
        self.validate()?;
        Ok(self
            .program_policies
            .iter()
            .flat_map(|record| {
                programs(&record.policy).into_iter().flat_map(|program| {
                    record.paths.iter().map(move |path| ProgramRequirement {
                        program: program.clone(),
                        path: *path,
                    })
                })
            })
            .collect())
    }

    pub(super) fn validate_program_policies(&self) -> Result<(), ArtifactErrorKind> {
        let invalid = |reason: String| ArtifactErrorKind::InvalidProgramPolicy(reason);
        validate_policy_sources(self.program_policies.iter().map(|record| &record.policy))
            .map_err(|error| invalid(error.to_string()))?;
        if self.program_policies.is_empty() {
            return Ok(());
        }
        let descriptor = self
            .descriptor
            .as_ref()
            .filter(|descriptor| descriptor.script_pubkey().is_p2tr())
            .ok_or_else(|| invalid("program policies require a Taproot descriptor".into()))?;
        let mut input = Input::default();
        descriptor
            .update_psbt_input(&mut input)
            .map_err(|error| invalid(error.to_string()))?;
        let mut seen = BTreeSet::new();
        for record in &self.program_policies {
            if !seen.insert(record) {
                return Err(invalid("duplicate program policy record".into()));
            }
            let programs = programs(&record.policy);
            if programs.is_empty() {
                return Err(invalid("record contains no emulated program".into()));
            }
            let script = validate_program_policy(&record.policy)
                .map_err(|error| invalid(error.to_string()))?;
            let parsed = Miniscript::<XOnlyPublicKey, Tap>::decode_consensus(&script).ok();
            let mut expected_paths = BTreeSet::new();
            if input
                .tap_scripts
                .values()
                .any(|(leaf, version)| leaf == &script && *version == LeafVersion::TapScript)
            {
                expected_paths.insert(ProgramSpendPath::ScriptPath(TapLeafHash::from_script(
                    &script,
                    LeafVersion::TapScript,
                )));
            }
            for program in programs {
                let key = program
                    .derive_public_key()
                    .map_err(|error| invalid(error.to_string()))?;
                if parsed
                    .as_ref()
                    .is_some_and(|policy| !policy.iter_pk().any(|candidate| candidate == key))
                {
                    return Err(invalid(
                        "program key is absent from its lowered policy".into(),
                    ));
                }
                let bare = Builder::new()
                    .push_x_only_key(&key)
                    .push_opcode(OP_CHECKSIG)
                    .into_script();
                if script == bare && input.tap_internal_key == Some(key) {
                    expected_paths.insert(ProgramSpendPath::KeyPath);
                }
            }
            if expected_paths.is_empty() || record.paths != expected_paths {
                return Err(invalid(
                    "program policy does not match its complete descriptor locations".into(),
                ));
            }
        }
        Ok(())
    }
}
