//! Portable descriptions of validated contract artifacts.

use super::{ArtifactError, CovenantRequirements, Object, ProgramPolicy, SupportedDescriptors};
use crate::contract::actions::TemplateKind;
use crate::template::FundingConstraints;
use crate::util::extended_address::ExtendedAddress;
use bitcoin::{hashes::sha256, ScriptBuf};
use sapio_base::effects::{EffectPath, PathFragment};
use sapio_base::policy::ScriptPolicy;
use sapio_base::serialization_helpers::SArc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A deterministic description of a complete validated artifact graph.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ArtifactExplanation {
    /// Whether any known script in this graph contains the native CTV opcode.
    /// This is an enforcement assumption, not a chain-activation assertion.
    pub native_ctv_in_graph: bool,
    /// Output occurrences in deterministic depth-first transaction order.
    pub nodes: Vec<ObjectExplanation>,
}

/// One occurrence of a contract output in the artifact graph.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ObjectExplanation {
    /// JSON pointer identifying this occurrence in the original artifact.
    /// The root pointer is empty; reused source paths still have distinct locations.
    pub location: String,
    /// Compilation source path; this can be reused by multiple output occurrences.
    pub source_path: SArc<EffectPath>,
    /// Address or script representation of this output.
    pub address: ExtendedAddress,
    /// Known descriptor and its actual spending branches.
    pub descriptor: Option<SupportedDescriptors>,
    /// Minimum value required at this contract input.
    pub required_input_sats: u64,
    /// Public lowering assumptions and predicates recorded by the compiler.
    pub covenants: CovenantRequirements,
    /// Exact program sources and their allowed signing locations.
    pub program_policies: Vec<ProgramPolicy>,
    /// Advertised action request paths and schemas; availability grants no authority.
    pub actions: Vec<ActionExplanation>,
    /// Committed and suggested transaction templates created from this output.
    pub templates: Vec<TemplateExplanation>,
}

/// A portable request interface advertised by a compiled contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ActionExplanation {
    /// Exact effect path to which the request must be attached.
    pub path: SArc<EffectPath>,
    /// Declared transaction mode when the path follows the compiler's convention.
    pub kind: Option<TemplateKind>,
    /// JSON request schema, or `None` for a local Rust-only callback.
    pub schema: Option<serde_json::Value>,
}

/// A template's fixed transaction fields and retained authoring requirements.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TemplateExplanation {
    /// Exact transaction-template commitment.
    pub hash: sha256::Hash,
    /// Whether this transaction is covenant-committed or merely suggested.
    pub kind: TemplateKind,
    /// Additional source authorization retained for a committed transaction.
    pub guards: Vec<ScriptPolicy>,
    /// Minimum aggregate funding, including the constructor's reserved fee.
    pub minimum_funding_sats: u64,
    /// Minimum contribution from this contract's input zero.
    pub contract_input_sats: u64,
    /// Fee explicitly reserved by the constructor.
    pub reserved_fee_sats: u64,
    /// Local spending constraints, separate from Script/evaluator enforcement.
    pub funding_constraints: Option<FundingConstraints>,
    /// Transaction version.
    pub version: i32,
    /// Consensus absolute locktime; values below 500000000 denote height.
    pub lock_time: u32,
    /// Inputs in committed transaction order.
    pub inputs: Vec<InputExplanation>,
    /// Outputs in committed transaction order.
    pub outputs: Vec<OutputExplanation>,
}

/// A transaction input and any declared funding role.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InputExplanation {
    /// Transaction input index.
    pub index: usize,
    /// Name supplied by a declarative transaction plan, if present.
    pub name: Option<String>,
    /// Required contribution, unknown for unannotated auxiliary input slots.
    pub minimum_sats: Option<u64>,
    /// Consensus sequence value, including relative-lock flags and units.
    pub sequence: u32,
}

/// An ordered output allocation and the child artifact it creates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OutputExplanation {
    /// Transaction output index.
    pub index: usize,
    /// Name supplied by a declarative transaction plan, if present.
    pub name: Option<String>,
    /// Exact committed value.
    pub amount_sats: u64,
    /// Exact committed destination script.
    pub script_pubkey: ScriptBuf,
    /// JSON pointer identifying the corresponding child in `nodes`.
    pub contract_location: String,
}

impl Object {
    /// Explain the validated graph without compiling contracts, contacting
    /// signers, or treating optional metadata as an execution requirement.
    ///
    /// Transaction construction, covenant lowering and local funding rules stay
    /// distinct. Spending capability, evaluator acceptance and chain maturity
    /// require additional evidence outside this artifact-only view.
    pub fn explain(&self) -> Result<ArtifactExplanation, ArtifactError> {
        self.validate()?;
        let mut nodes = Vec::new();
        let mut pending = vec![(String::new(), self)];
        while let Some((location, object)) = pending.pop() {
            let mut children = Vec::new();
            let mut templates = Vec::new();
            for (kind, field, hash, template) in object
                .ctv_to_tx
                .iter()
                .map(|(hash, template)| {
                    (
                        TemplateKind::Committed,
                        "template_hash_to_template_map",
                        hash,
                        template,
                    )
                })
                .chain(object.suggested_txs.iter().map(|(hash, template)| {
                    (
                        TemplateKind::Suggested,
                        "suggested_template_hash_to_template_map",
                        hash,
                        template,
                    )
                }))
            {
                let funding = template.funding_constraints.as_ref();
                let inputs = template
                    .tx
                    .input
                    .iter()
                    .enumerate()
                    .map(|(index, input)| {
                        let requirement = funding.map(|rules| &rules.inputs[index]);
                        InputExplanation {
                            index,
                            name: requirement.map(|requirement| requirement.name.clone()),
                            minimum_sats: requirement
                                .map(|requirement| requirement.minimum.to_sat())
                                .or_else(|| {
                                    (index == 0).then(|| template.required_input_amount.to_sat())
                                }),
                            sequence: input.sequence.to_consensus_u32(),
                        }
                    })
                    .collect();
                let outputs = template
                    .tx
                    .output
                    .iter()
                    .enumerate()
                    .map(|(index, output)| {
                        let child_location = format!(
                            "{location}/{field}/{hash}/outputs_info/{index}/receiving_contract"
                        );
                        children.push((child_location.clone(), &template.outputs[index].contract));
                        OutputExplanation {
                            index,
                            name: funding.map(|rules| rules.outputs[index].clone()),
                            amount_sats: output.value.to_sat(),
                            script_pubkey: output.script_pubkey.clone(),
                            contract_location: child_location,
                        }
                    })
                    .collect();
                templates.push(TemplateExplanation {
                    hash: *hash,
                    kind,
                    guards: template.guards.clone(),
                    minimum_funding_sats: template.max.to_sat(),
                    contract_input_sats: template.required_input_amount.to_sat(),
                    reserved_fee_sats: (template.max - template.total_amount()).to_sat(),
                    funding_constraints: template.funding_constraints.clone(),
                    version: template.tx.version.0,
                    lock_time: template.tx.lock_time.to_consensus_u32(),
                    inputs,
                    outputs,
                });
            }
            nodes.push(ObjectExplanation {
                location,
                source_path: object.root_path.clone(),
                address: object.address.clone(),
                descriptor: object.descriptor.clone(),
                required_input_sats: object.required_input_amount.to_sat(),
                covenants: object.covenant_requirements.clone(),
                program_policies: object.program_policies.clone(),
                actions: object
                    .continue_apis
                    .iter()
                    .map(|(path, point)| ActionExplanation {
                        path: path.clone(),
                        kind: match path.0.iter().next() {
                            Some(PathFragment::Next) => Some(TemplateKind::Committed),
                            Some(PathFragment::Suggested) => Some(TemplateKind::Suggested),
                            _ => None,
                        },
                        schema: point
                            .schema
                            .as_ref()
                            .map(|schema| schema.0.as_ref().clone()),
                    })
                    .collect(),
                templates,
            });
            pending.extend(children.into_iter().rev());
        }
        Ok(ArtifactExplanation {
            native_ctv_in_graph: self.requires_native_ctv(),
            nodes,
        })
    }
}
