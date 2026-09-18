// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Object is the output of Sapio Compilation & can be linked to a specific coin

pub mod error;
pub use error::*;
pub mod bind;
pub mod descriptors;
mod enforcement;
mod explain;
pub use explain::{
    ActionExplanation, ArtifactExplanation, InputExplanation, ObjectExplanation, OutputExplanation,
    TemplateExplanation,
};
mod programs;
pub use programs::{ProgramPolicy, ProgramRequirement};
pub mod taproot;
mod validation;
use crate::contract::abi::continuation::ContinuationPoint;
use crate::contract::CompilationError;
use crate::template::Template;
use crate::util::extended_address::ExtendedAddress;
use bitcoin::hashes::sha256;
pub use descriptors::*;
use sapio_base::covenant::{Ctv, LoweringPlan};
use sapio_base::effects::EffectPath;
use sapio_base::effects::PathFragment;
use sapio_base::miniscript::*;
use sapio_base::policy::ScriptPolicy;
use sapio_base::serialization_helpers::SArc;
use sapio_base::simp::CompiledObjectLT;
use sapio_base::simp::SIMPAttachableAt;
use sapio_base::simp::SIMPError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
pub use taproot::{RawTaproot, RawTaprootError};
pub use validation::{ArtifactError, ArtifactErrorKind};

/// Metadata attached to a particular guard policy.
#[derive(Serialize, Deserialize, Clone, JsonSchema, Debug, PartialEq, Eq)]
pub struct GuardMetadata {
    /// The complete policy source which produced the metadata.
    pub policy: ScriptPolicy,
    /// Distinct values per protocol, in attachment order.
    pub protocols: BTreeMap<i64, Vec<serde_json::Value>>,
}

/// Metadata for Object, arbitrary KV set.
#[derive(Serialize, Deserialize, Clone, JsonSchema, Debug, PartialEq, Eq, Default)]
pub struct ObjectMetadata {
    /// Additional non-standard fields for future upgrades
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
    /// SIMP: Sapio Interactive Metadata Protocol
    pub simp: BTreeMap<i64, serde_json::Value>,
    /// Guard records in deterministic policy order. Each protocol retains its
    /// distinct values in attachment order.
    pub simps_for_guards: Vec<GuardMetadata>,
}
impl ObjectMetadata {
    /// Is there any metadata in this field?
    pub fn is_empty(&self) -> bool {
        *self == Default::default()
    }

    /// attempts to add a SIMP to the object meta.
    ///
    /// Returns [`SIMPError::AlreadyDefined`] if one was previously set.
    pub fn add_simp<S: SIMPAttachableAt<CompiledObjectLT>>(
        mut self,
        s: S,
    ) -> Result<Self, SIMPError> {
        let old = self.simp.insert(s.get_protocol_number(), s.to_json()?);
        if let Some(old) = old {
            Err(SIMPError::AlreadyDefined(old))
        } else {
            Ok(self)
        }
    }

    pub(crate) fn add_guard_simps(
        mut self,
        all_guard_simps: BTreeMap<
            ScriptPolicy,
            Vec<Arc<dyn SIMPAttachableAt<sapio_base::simp::GuardLT>>>,
        >,
    ) -> Result<ObjectMetadata, CompilationError> {
        if !self.simps_for_guards.is_empty() {
            return Err(CompilationError::Custom(
                "Contract metadata cannot prepopulate guard SIMPs".into(),
            ));
        }
        for (policy, metadata) in all_guard_simps {
            let mut protocols: BTreeMap<i64, Vec<serde_json::Value>> = BTreeMap::new();
            for simp in metadata {
                let value = simp
                    .to_json()
                    .map_err(CompilationError::SerializationError)?;
                let values = protocols.entry(simp.get_protocol_number()).or_default();
                if !values.contains(&value) {
                    values.push(value);
                }
            }
            if !protocols.is_empty() {
                self.simps_for_guards
                    .push(GuardMetadata { policy, protocols });
            }
        }
        Ok(self)
    }
}

/// Explicit predicates resolved using public, deterministic lowering inputs.
/// These records include wrapped finish and continuation guards, not only
/// automatically generated CTV predicates on committed templates.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub struct CovenantRequirements {
    /// Public compilation inputs; contains no signer transport or host callback.
    pub lowering: LoweringPlan,
    /// Every wrapped predicate resolved while compiling this contract.
    pub predicates: BTreeSet<Ctv>,
}

impl Default for CovenantRequirements {
    fn default() -> Self {
        Self {
            lowering: LoweringPlan::Native,
            predicates: BTreeSet::new(),
        }
    }
}

/// One original authorization branch for a committed transaction. Keeping the
/// action/template boundary reproduces Script compilation before deduplication.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct CommittedPolicy {
    /// Complete action guard, before covenant lowering.
    pub action_guard: ScriptPolicy,
    /// Additional template guards, before covenant injection and lowering.
    pub template_guards: Vec<ScriptPolicy>,
}

/// Object holds a contract's complete context required post-compilation
/// Public fields and deserialization can produce inconsistent objects. Call
/// [`Object::validate`] before using an artifact; binding performs this check.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub struct Object {
    /// Original complete guard alternatives for each committed template, before
    /// catalog deduplication merges guards. Each branch also requires the
    /// template's covenant. Retained to reproduce its exact descriptor script.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub committed_policy_guards: BTreeMap<sha256::Hash, BTreeSet<CommittedPolicy>>,
    /// Complete finish and suggested-action policies, independently of the
    /// committed templates. These alternatives can authorize other spends.
    /// Validation binds them to the descriptor, but cannot authenticate their
    /// origin or decide whether their keys/policies are acceptable to a funder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternative_policies: Vec<ScriptPolicy>,
    /// Complete program-bearing source branches and their validated signing locations.
    pub program_policies: Vec<ProgramPolicy>,
    /// Reproducible covenant lowering and the predicates it resolved.
    pub covenant_requirements: CovenantRequirements,
    /// CTV-protected templates, deduplicated only when their binding payloads
    /// agree. Each stored template retains all alternative preconditions.
    #[serde(
        rename = "template_hash_to_template_map",
        skip_serializing_if = "BTreeMap::is_empty",
        default
    )]
    pub ctv_to_tx: BTreeMap<sha256::Hash, Template>,
    /// a map of template hashes to the corresponding template, that in the
    /// policy are not necessarily CTV protected but we might want to know about
    /// anyways.
    #[serde(
        rename = "suggested_template_hash_to_template_map",
        skip_serializing_if = "BTreeMap::is_empty",
        default
    )]
    pub suggested_txs: BTreeMap<sha256::Hash, Template>,
    /// A Map of arguments to continue execution and generate an update at this
    /// point via a passed message
    #[serde(
        rename = "continuation_points",
        skip_serializing_if = "BTreeMap::is_empty",
        default
    )]
    pub continue_apis: BTreeMap<SArc<EffectPath>, ContinuationPoint>,
    /// The base location for the set of continue_apis.
    pub root_path: SArc<EffectPath>,
    /// The Object's address, or a ScriptBuf if no address is possible
    pub address: ExtendedAddress,
    /// The Object's descriptor -- if there is one known/available
    #[serde(
        rename = "known_descriptor",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub descriptor: Option<SupportedDescriptors>,
    /// Minimum satoshis required at this contract's input, including its
    /// declared floor and every committed or suggested template requirement.
    /// Auxiliary inputs fund their separately declared contributions.
    #[serde(
        rename = "required_input_amount_sats",
        with = "bitcoin::amount::serde::as_sat"
    )]
    #[schemars(with = "u64")]
    pub required_input_amount: bitcoin::Amount,
    /// metadata generated for this contract
    pub metadata: ObjectMetadata,
}

impl Object {
    /// Create an address destination with an explicit minimum funding amount.
    /// Use zero when the destination imposes no minimum of its own.
    pub fn from_address(
        address: bitcoin::Address,
        required_input_amount: bitcoin::Amount,
    ) -> Object {
        Object {
            committed_policy_guards: BTreeMap::new(),
            alternative_policies: Vec::new(),
            program_policies: Vec::new(),
            covenant_requirements: CovenantRequirements::default(),
            ctv_to_tx: BTreeMap::new(),
            suggested_txs: BTreeMap::new(),
            continue_apis: Default::default(),
            root_path: SArc(EffectPath::push(
                None,
                PathFragment::Named(SArc(Arc::new("".into()))),
            )),
            address: address.into(),
            descriptor: None,
            required_input_amount,
            metadata: Default::default(),
        }
    }

    /// Create a recognized script destination with an explicit funding minimum.
    pub fn from_script(
        script: bitcoin::ScriptBuf,
        required_input_amount: bitcoin::Amount,
        net: bitcoin::Network,
    ) -> Result<Object, ObjectError> {
        bitcoin::Address::from_script(&script, net)
            .map_err(|_| ObjectError::UnknownScriptType(script.clone()))
            .map(|m| Object::from_address(m, required_input_amount))
    }
    /// create an op_return of no more than 40 bytes
    pub fn from_op_return<'a, I: ?Sized>(data: &'a I) -> Result<Object, ObjectError>
    where
        &'a [u8]: From<&'a I>,
    {
        Ok(Object {
            committed_policy_guards: BTreeMap::new(),
            alternative_policies: Vec::new(),
            program_policies: Vec::new(),
            covenant_requirements: CovenantRequirements::default(),
            ctv_to_tx: BTreeMap::new(),
            suggested_txs: BTreeMap::new(),
            continue_apis: Default::default(),
            root_path: SArc(EffectPath::push(
                None,
                PathFragment::Named(SArc(Arc::new("".into()))),
            )),
            address: ExtendedAddress::make_op_return(data)?,
            descriptor: None,
            required_input_amount: bitcoin::Amount::ZERO,
            metadata: Default::default(),
        })
    }

    /// Create a descriptor destination with an explicit funding minimum.
    pub fn from_descriptor<T>(d: Descriptor<T>, required_input_amount: bitcoin::Amount) -> Self
    where
        Descriptor<T>: Into<SupportedDescriptors>,
        T: MiniscriptKey + ToPublicKey,
    {
        Object {
            committed_policy_guards: BTreeMap::new(),
            alternative_policies: Vec::new(),
            program_policies: Vec::new(),
            covenant_requirements: CovenantRequirements::default(),
            ctv_to_tx: BTreeMap::new(),
            suggested_txs: BTreeMap::new(),
            continue_apis: Default::default(),
            root_path: SArc(EffectPath::push(
                None,
                PathFragment::Named(SArc(Arc::new("".into()))),
            )),
            address: d.address(bitcoin::Network::Bitcoin).unwrap().into(),
            descriptor: Some(d.into()),
            required_input_amount,
            metadata: Default::default(),
        }
    }
}
