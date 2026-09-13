//! Inspect complete spending alternatives without producing signatures or evidence.
//!
//! Asset declarations are capabilities, not authorization. A plan is neither a
//! successful evaluator invocation nor proof of chain maturity. Existing PSBT
//! signatures are reported as unverified; normal finalization remains mandatory.

use super::{
    prepare_program_request, ArtifactProgramError, ProgramError, ProgramSigningRequest,
    ProgramSpendPath, MAX_WITNESS_BYTES,
};
use bitcoin::hashes::{hash160, ripemd160, sha256, sha256d, Hash};
use bitcoin::hex::DisplayHex;
use bitcoin::psbt::{Input, Psbt};
use bitcoin::taproot::{ControlBlock, LeafVersion, TapLeafHash};
use bitcoin::{absolute, relative, PublicKey, ScriptBuf, Weight, XOnlyPublicKey};
use sapio::contract::abi::object::{Object, ProgramRequirement, SupportedDescriptors};
use sapio_base::miniscript::miniscript::satisfy::{Placeholder, Witness};
use sapio_base::miniscript::plan::AssetProvider;
use sapio_base::miniscript::psbt::PsbtInputSatisfier;
use sapio_base::miniscript::{
    self, DefiniteDescriptorKey, Descriptor, Miniscript, Satisfier, Tap, ToPublicKey,
};
use sapio_base::CTVHash;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

// Match the compiler's 65,536 expanded-operand work scale without coupling a
// host spending service to compiler-internal constants. This is one aggregate
// budget for the report, charging policy size for discovery and every solver
// pass. It bounds input work; Miniscript may internally revisit some nodes.
const MAX_PLANNING_WORK: usize = 65_536;
const WORK_LIMIT: &str = "Native satisfaction planning exceeds 65,536 policy-node/pass work units";

struct PlanningWork(Cell<usize>);
impl PlanningWork {
    fn charge(&self, amount: usize) -> Result<(), &'static str> {
        match self.0.get().checked_sub(amount) {
            Some(remaining) => {
                self.0.set(remaining);
                Ok(())
            }
            None => {
                self.0.set(0);
                Err(WORK_LIMIT)
            }
        }
    }
}

/// Exact preimage required by a selected satisfaction. Miniscript uses 32 bytes.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub enum HashRequirement {
    /// A single SHA256 digest.
    Sha256(sha256::Hash),
    /// A double SHA256 digest, in consensus byte order.
    Hash256(sha256d::Hash),
    /// A RIPEMD160 digest.
    Ripemd160(ripemd160::Hash),
    /// A SHA256 followed by RIPEMD160 digest.
    Hash160(hash160::Hash),
}

/// A caller-configured evidence adapter and signer for exactly one program slot.
///
/// The codec identifies the caller's adapter, not an interpretation supplied by
/// SIMP. The complete program (including parameters and root) and spending path
/// must match. Declaring evidence available does not evaluate its predicate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProgramCapability {
    /// The exact program and signature location handled by this adapter.
    pub requirement: ProgramRequirement,
    /// An explicit, application-defined evidence codec name.
    pub codec: String,
    /// Whether the caller can supply evidence using this codec.
    pub evidence_available: bool,
    /// Whether the caller has explicitly configured a signer for this program.
    pub signer_available: bool,
}

/// A codec's already-constructed evidence for one exact program signature slot.
///
/// The planner only borrows the bytes. Implementations must expose fixed public
/// identity and evidence; preparing a spend never invokes signing or transport.
/// The evaluator, when explicitly invoked later, decides whether the bytes prove
/// its predicate. Neither the codec name nor an adapter grants authorization.
pub trait ProgramEvidenceAdapter {
    /// Exact evaluator/program/parameters/root and selected signature path.
    fn requirement(&self) -> &ProgramRequirement;
    /// Application-defined codec matching an explicitly configured capability.
    fn codec(&self) -> &str;
    /// Already-constructed auxiliary evidence; no transaction witness is implied.
    fn witness(&self) -> &[u8];
}

/// Portable evidence value implementing the explicit adapter boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProgramEvidence {
    /// Full program identity and exact path being authorized.
    pub requirement: ProgramRequirement,
    /// Explicit evidence codec; never inferred from SIMP or program metadata.
    pub codec: String,
    /// Auxiliary bytes, bounded again at request preparation.
    #[serde(deserialize_with = "super::deserialize_witness")]
    #[schemars(length(max = "MAX_WITNESS_BYTES"))]
    pub witness: Vec<u8>,
}

impl ProgramEvidenceAdapter for ProgramEvidence {
    fn requirement(&self) -> &ProgramRequirement {
        &self.requirement
    }
    fn codec(&self) -> &str {
        &self.codec
    }
    fn witness(&self) -> &[u8] {
        &self.witness
    }
}

/// Explicit local capabilities used to choose a complete native satisfaction.
///
/// Keys imply ability to request a signature, not that signing is authorized.
/// Program keys cannot be enabled through the ordinary Schnorr-key set; they
/// require their complete program capability or an existing PSBT signature.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SpendAssets {
    /// Ordinary Schnorr keys the caller can use.
    pub schnorr_keys: BTreeSet<XOnlyPublicKey>,
    /// Ordinary ECDSA keys the caller can use.
    pub ecdsa_keys: BTreeSet<PublicKey>,
    /// Preimages the caller can supply later.
    pub preimages: BTreeSet<HashRequirement>,
    /// Explicit program adapters and signer capabilities.
    pub programs: Vec<ProgramCapability>,
}

/// Availability of one item, without conflating capability with verification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Availability {
    /// Neither supplied nor declared available.
    Missing,
    /// Available from an explicit capability or the validated artifact.
    CanProvide,
    /// Present in the PSBT; its signature has not been verified here.
    PresentUnverified,
    /// Present and its content was checked (preimages and descriptor proofs).
    PresentVerified,
}

impl Availability {
    fn available(self) -> bool {
        self != Self::Missing
    }
}

/// A check has three outcomes so absent transaction/chain data stays visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum PlanCheck {
    /// The supplied data meets the constraint.
    Met,
    /// The supplied data contradicts the constraint.
    Unmet,
    /// Required data was not supplied or this planner cannot establish the fact.
    Unknown,
}

/// One concrete location whose full witness can be considered independently.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SpendPath {
    /// Taproot's tweaked internal-key spend.
    KeyPath,
    /// A Taproot leaf, identified by its exact hash.
    ScriptPath(TapLeafHash),
    /// A non-Taproot descriptor's selected satisfaction.
    Descriptor,
}

/// One required item in the selected native satisfaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SpendRequirement {
    /// An ordinary signature in this branch.
    SchnorrSignature {
        /// The untweaked public key, or script key for a leaf.
        key: XOnlyPublicKey,
        /// Signature presence or an explicit signing capability.
        availability: Availability,
    },
    /// A signature in an ECDSA descriptor.
    EcdsaSignature {
        /// The complete public key.
        key: PublicKey,
        /// Signature presence or an explicit signing capability.
        availability: Availability,
    },
    /// An exact program signature plus its independently declared evidence adapter.
    Program {
        /// Program identity, public root and selected signature location.
        requirement: ProgramRequirement,
        /// Existing signature or capability to request one with available evidence.
        signature: Availability,
        /// Explicit adapter codec, absent when no adapter was configured.
        codec: Option<String>,
        /// Evidence capability only; the evaluator is never run by this planner.
        evidence: Availability,
        /// Whether a signer was explicitly configured for this program.
        signer_available: bool,
    },
    /// A 32-byte native hashlock preimage.
    Preimage {
        /// Exact digest and algorithm.
        hash: HashRequirement,
        /// Presence with hash verification, or declared capability.
        availability: Availability,
    },
    /// A native CTV predicate executed by the selected witness.
    NativeTemplateHash {
        /// Exact BIP119 commitment for the selected input.
        hash: sha256::Hash,
        /// Checks the candidate with all supplied finalized scriptSig fields.
        transaction: PlanCheck,
    },
    /// The selected branch requires an absolute locktime.
    AbsoluteTimelock {
        /// Bitcoin's consensus locktime value; values >= 500000000 are time.
        value: u32,
        /// Checks the transaction's locktime and sequence enablement.
        transaction: PlanCheck,
        /// This API has no chain clock, so chain maturity remains unknown.
        chain_maturity: PlanCheck,
    },
    /// The selected branch requires a relative locktime.
    RelativeTimelock {
        /// Bitcoin's consensus sequence locktime value, including the time flag.
        value: u32,
        /// Checks transaction version, sequence flags, units and value.
        transaction: PlanCheck,
        /// This API has no coin confirmation age, so maturity remains unknown.
        chain_maturity: PlanCheck,
    },
    /// Script and control block required for this exact leaf.
    TaprootProof {
        /// The leaf authenticated by the proof.
        leaf: TapLeafHash,
        /// The proof is already present or reconstructible from the artifact.
        availability: Availability,
    },
}

/// Whether a complete native witness template could be selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum BranchStatus {
    /// All selected assets are present or declared available. This is not a
    /// signature check, evaluator success, relay-policy check or maturity proof.
    Planned,
    /// A complete hypothetical witness exists but some selected assets are missing.
    MissingAssets,
    /// No non-malleable satisfaction fits the supplied transaction fields.
    IncompatibleTransaction,
    /// The script or descriptor has no supported non-malleable witness plan.
    Unsupported,
}

/// A selected witness element, in the exact order returned by Miniscript.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WitnessItem {
    /// Miniscript's unambiguous placeholder description; contains no secret data.
    pub description: String,
    /// Maximum serialized bytes including the witness-length or script-push prefix.
    pub serialized_bytes: u64,
}

/// One complete candidate satisfaction for a key path, leaf or ECDSA descriptor.
///
/// Each leaf uses Miniscript's smallest available non-malleable satisfaction;
/// if assets are missing, it reports one hypothetical completion. Alternatives
/// within one leaf are not exhaustively enumerated. No branch is auto-authorized.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BranchPlan {
    /// Explicit branch selector; the caller chooses which plan to use.
    pub path: SpendPath,
    /// Native Miniscript expression, or an explanation for unsupported scripts.
    pub policy: String,
    /// Outcome of planning, separate from actual spend validity.
    pub status: BranchStatus,
    /// Complete required assets and native lock requirements for the chosen witness.
    pub requirements: Vec<SpendRequirement>,
    /// The selected stack layout. For legacy descriptors this becomes scriptSig.
    pub witness_template: Vec<WitnessItem>,
    /// Upper bound for this input's witness and serialized scriptSig, including
    /// their length prefixes. Excludes outpoint, sequence and transaction overhead.
    /// It assumes the selected layout and the PSBT's current annex are retained.
    #[schemars(with = "Option<u64>")]
    pub satisfaction_weight_upper_bound: Option<Weight>,
    /// Upper bound for the serialized witness only. Zero for legacy descriptors.
    pub witness_bytes_upper_bound: Option<u64>,
    /// Whether some completion of this leaf fits the supplied transaction fields.
    pub transaction_compatible: PlanCheck,
}

/// Read-only plan for spending one validated artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SpendReport {
    /// All independent output-level spending locations, in deterministic order.
    pub branches: Vec<BranchPlan>,
    /// Inputs without a previous output. Known previous transactions are checked.
    pub missing_prevouts: Vec<usize>,
    /// The selected funding input matches the artifact script and minimum value.
    pub funding: PlanCheck,
    /// Retained local requirements for an exact catalogued template at input zero.
    /// An uncatalogued transaction does not acquire inferred funding policy.
    pub template_funding: Option<sapio::template::funding::FundingReport>,
    /// Bytes actually present in a final witness, independent of any planned bound.
    /// Observation does not validate that witness or match it to a branch.
    pub observed_final_witness_bytes: Option<u64>,
}

/// One explicitly selected branch and its unsigned program requests.
#[derive(Debug)]
pub struct PreparedSpend {
    /// Freshly computed full-branch requirements; native assets may be pending.
    pub plan: BranchPlan,
    /// Funding and existing native assets, with descriptor proofs supplied.
    pub psbt: Psbt,
    /// Only the selected witness's unsigned program slots, in witness order.
    /// Callers explicitly submit requests and merge verified responses.
    pub program_requests: Vec<ProgramSigningRequest>,
}

/// Invalid artifacts, conflicting PSBT data or malformed explicit capabilities.
#[derive(Debug)]
pub enum SpendPlanError {
    /// The artifact and spending input disagree, or the artifact is invalid.
    Artifact(ArtifactProgramError),
    /// Supplied previous-output evidence is structurally invalid or conflicting.
    Funding(sapio_base::psbt::FundingError),
    /// A capability has an empty/oversized codec, duplicate slot or unknown program.
    InvalidCapabilities,
    /// The requested path does not occur in the validated artifact.
    UnknownPath,
    /// This path has no supported non-malleable witness template.
    UnsupportedBranch,
    /// The selected branch contradicts the transaction's fixed fields.
    IncompatibleTransaction,
    /// Preparing a spend requires all input amounts and scripts.
    MissingPrevouts(Vec<usize>),
    /// Evidence for a selected unsigned program was not supplied.
    MissingEvidence(ProgramRequirement),
    /// An evidence adapter has the wrong identity, codec, or duplicate slot.
    InvalidEvidence,
    /// An existing witness or redeem script disagrees with the native descriptor.
    ConflictingScripts,
}

impl fmt::Display for SpendPlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Artifact(error) => error.fmt(f),
            Self::Funding(error) => error.fmt(f),
            Self::InvalidCapabilities => f.write_str("program capabilities require a unique artifact requirement and a codec of 1..=128 bytes"),
            Self::UnknownPath => f.write_str("selected spending path is absent from the artifact"),
            Self::UnsupportedBranch => f.write_str("selected branch has no supported non-malleable spending plan"),
            Self::IncompatibleTransaction => f.write_str("selected branch contradicts the transaction's fixed fields"),
            Self::MissingPrevouts(inputs) => write!(f, "previous outputs are missing for inputs {inputs:?}"),
            Self::MissingEvidence(requirement) => write!(f, "missing evidence for program {} at {:?}", requirement.program.instance().id().0, requirement.path),
            Self::InvalidEvidence => f.write_str("evidence must match one selected program requirement and its explicit codec"),
            Self::ConflictingScripts => f.write_str("PSBT witness or redeem script conflicts with the artifact descriptor"),
        }
    }
}
impl std::error::Error for SpendPlanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Artifact(error) => Some(error),
            Self::Funding(error) => Some(error),
            _ => None,
        }
    }
}
impl From<ArtifactProgramError> for SpendPlanError {
    fn from(error: ArtifactProgramError) -> Self {
        Self::Artifact(error)
    }
}
impl From<ProgramError> for SpendPlanError {
    fn from(error: ProgramError) -> Self {
        Self::Artifact(error.into())
    }
}

/// Inspect complete spending alternatives without signing or evaluating programs.
///
/// Optional PSBT data is checked before it influences the report. Every supplied
/// non-witness previous transaction must authenticate its outpoint; witness-only
/// previous outputs remain caller assertions. Missing prevouts and chain maturity
/// are reported explicitly. An existing signature is never treated as verified.
pub fn plan_spends(
    object: &Object,
    psbt: Option<(&Psbt, usize)>,
    assets: &SpendAssets,
) -> Result<SpendReport, SpendPlanError> {
    let programs = object
        .program_requirements()
        .map_err(ArtifactProgramError::Artifact)?;
    let mut configured = BTreeSet::new();
    for capability in &assets.programs {
        if capability.codec.is_empty()
            || capability.codec.len() > 128
            || !configured.insert(&capability.requirement)
            || !programs.contains(&capability.requirement)
        {
            return Err(SpendPlanError::InvalidCapabilities);
        }
    }
    let mut program_keys: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for requirement in &programs {
        let key = requirement
            .program
            .derive_public_key()
            .map_err(ProgramError::Instance)?;
        program_keys
            .entry((key, requirement.path))
            .or_default()
            .push(requirement);
    }
    let expected = spending_metadata(object)?;
    let (missing_prevouts, funding) = validate_input(object, psbt, &expected)?;
    // Match PsbtInputSatisfier's BIP119 view, including finalized scriptSigs
    // from every input. Hash once so each native predicate costs one comparison.
    let actual_template = psbt.map(|(psbt, index)| {
        let mut candidate = psbt.unsigned_tx.clone();
        for (input, metadata) in candidate.input.iter_mut().zip(&psbt.inputs) {
            if let Some(script_sig) = &metadata.final_script_sig {
                input.script_sig = script_sig.clone();
            }
        }
        candidate.get_ctv_hash(index as u32)
    });
    let template_funding = psbt
        .filter(|(_, index)| *index == 0)
        .and_then(|(psbt, _)| {
            let hash = psbt.unsigned_tx.get_ctv_hash(0);
            object
                .ctv_to_tx
                .get(&hash)
                .or_else(|| object.suggested_txs.get(&hash))
                .map(|template| template.check_funded_psbt(psbt))
        })
        .transpose()
        .map_err(ArtifactProgramError::Funding)?;
    let work = PlanningWork(Cell::new(MAX_PLANNING_WORK));
    let provider = Provider {
        assets,
        psbt,
        program_keys: &program_keys,
        hypothetical_assets: false,
        hypothetical_transaction: psbt.is_none(),
        hypothetical_template: None,
        actual_template,
        work: &work,
    };
    let mut report = SpendReport {
        branches: Vec::new(),
        missing_prevouts,
        funding,
        template_funding,
        observed_final_witness_bytes: psbt.and_then(|(psbt, index)| {
            psbt.inputs[index]
                .final_script_witness
                .as_ref()
                .map(|witness| witness.size() as u64)
        }),
    };
    if let Some(key) = expected.tap_internal_key {
        report.branches.push(key_plan(key, &provider));
        // A duplicate leaf may have several control blocks. The shortest proof
        // gives a sound bound for the concrete proof displayed by this plan.
        let mut leaves = std::collections::BTreeMap::new();
        for (control, (script, version)) in &expected.tap_scripts {
            let leaf = TapLeafHash::from_script(script, *version);
            let previous = leaves.entry(leaf).or_insert((script, control));
            if control.size() < previous.1.size() {
                *previous = (script, control);
            }
        }
        for (leaf, (script, control)) in leaves {
            let native = match &object.descriptor {
                Some(SupportedDescriptors::XOnly(Descriptor::Tr(tree))) => tree
                    .leaves()
                    .find(|candidate| candidate.miniscript().encode() == *script)
                    .map(|candidate| candidate.miniscript().clone()),
                _ => None,
            };
            report.branches.push(tap_plan(
                leaf,
                script,
                control,
                native.as_deref(),
                &provider,
            ));
        }
    } else if let Some(SupportedDescriptors::Pk(descriptor)) = &object.descriptor {
        report
            .branches
            .push(descriptor_plan(&descriptor.to_string(), &provider));
    } else {
        report.branches.push(unsupported(
            SpendPath::Descriptor,
            "No supported spending descriptor".into(),
        ));
    }
    Ok(report)
}

/// Prepare a caller-selected full branch using exact, explicitly supplied evidence.
///
/// This recomputes the plan against validated funding instead of trusting a
/// previously serialized report. Native signatures/preimages may remain pending.
/// Every selected unsigned program needs an adapter matching its full identity
/// and declared codec. The returned requests have not been evaluated or signed.
/// Unused adapters are rejected so evidence cannot silently select other branches.
pub fn prepare_spend(
    object: &Object,
    path: SpendPath,
    mut psbt: Psbt,
    input_index: usize,
    assets: &SpendAssets,
    evidence: &[impl ProgramEvidenceAdapter],
) -> Result<PreparedSpend, SpendPlanError> {
    let mut planning_assets = assets.clone();
    let mut seen = BTreeSet::new();
    for adapter in evidence {
        if !seen.insert(adapter.requirement())
            || adapter.codec().is_empty()
            || adapter.codec().len() > 128
        {
            return Err(SpendPlanError::InvalidEvidence);
        }
        if adapter.witness().len() > MAX_WITNESS_BYTES {
            return Err(ProgramError::WitnessTooLarge(adapter.witness().len()).into());
        }
        let Some(capability) = planning_assets
            .programs
            .iter_mut()
            .find(|capability| &capability.requirement == adapter.requirement())
        else {
            return Err(SpendPlanError::InvalidEvidence);
        };
        if capability.codec != adapter.codec() {
            return Err(SpendPlanError::InvalidEvidence);
        }
        capability.evidence_available = true;
    }
    let report = plan_spends(object, Some((&psbt, input_index)), &planning_assets)?;
    if !report.missing_prevouts.is_empty() {
        return Err(SpendPlanError::MissingPrevouts(report.missing_prevouts));
    }
    let plan = report
        .branches
        .into_iter()
        .find(|plan| plan.path == path)
        .ok_or(SpendPlanError::UnknownPath)?;
    match plan.status {
        BranchStatus::Unsupported => return Err(SpendPlanError::UnsupportedBranch),
        BranchStatus::IncompatibleTransaction => {
            return Err(SpendPlanError::IncompatibleTransaction)
        }
        _ => (),
    }
    let selected: Vec<_> = plan
        .requirements
        .iter()
        .filter_map(|requirement| match requirement {
            SpendRequirement::Program {
                requirement,
                signature,
                ..
            } if *signature != Availability::PresentUnverified => Some(requirement),
            _ => None,
        })
        .collect();
    for adapter in evidence {
        if !selected.contains(&adapter.requirement()) {
            return Err(SpendPlanError::InvalidEvidence);
        }
    }
    let mut requests = Vec::with_capacity(selected.len());
    for requirement in selected {
        let adapter = evidence
            .iter()
            .find(|adapter| adapter.requirement() == requirement)
            .ok_or_else(|| SpendPlanError::MissingEvidence(requirement.clone()))?;
        let request = prepare_program_request(
            object,
            requirement,
            psbt.clone(),
            input_index as u32,
            adapter.witness().to_vec(),
        )?;
        psbt = request.psbt.0.clone();
        requests.push(request);
    }
    if requests.is_empty() {
        let expected = spending_metadata(object)?;
        let input = &mut psbt.inputs[input_index];
        input.tap_internal_key = expected.tap_internal_key;
        input.tap_merkle_root = expected.tap_merkle_root;
        input.tap_scripts.extend(expected.tap_scripts);
        input.witness_script = expected.witness_script;
        input.redeem_script = expected.redeem_script;
    }
    Ok(PreparedSpend {
        plan,
        psbt,
        program_requests: requests,
    })
}

fn spending_metadata(object: &Object) -> Result<Input, SpendPlanError> {
    let mut expected = Input::default();
    match &object.descriptor {
        Some(SupportedDescriptors::Pk(descriptor)) => match descriptor {
            Descriptor::Bare(_) | Descriptor::Pkh(_) | Descriptor::Wpkh(_) => (),
            Descriptor::Wsh(wsh) => expected.witness_script = Some(wsh.inner_script()),
            Descriptor::Sh(sh) => match sh.as_inner() {
                miniscript::descriptor::ShInner::Wsh(wsh) => {
                    expected.witness_script = Some(wsh.inner_script());
                    expected.redeem_script = Some(wsh.inner_script().to_p2wsh());
                }
                _ => expected.redeem_script = Some(sh.inner_script()),
            },
            Descriptor::Tr(_) => return Err(SpendPlanError::UnsupportedBranch),
        },
        Some(descriptor) => descriptor
            .update_psbt_input(&mut expected)
            .map_err(ArtifactProgramError::Descriptor)?,
        None => (),
    }
    Ok(expected)
}

fn validate_input(
    object: &Object,
    psbt: Option<(&Psbt, usize)>,
    expected: &Input,
) -> Result<(Vec<usize>, PlanCheck), SpendPlanError> {
    let Some((psbt, selected)) = psbt else {
        return Ok((vec![], PlanCheck::Unknown));
    };
    sapio_psbt::validate_psbt(psbt).map_err(ProgramError::Psbt)?;
    let prevouts = sapio_base::psbt::previous_outputs(psbt).map_err(SpendPlanError::Funding)?;
    if selected >= psbt.inputs.len() {
        return Err(ProgramError::InputIndex(selected as u32).into());
    }
    let missing: Vec<_> = prevouts
        .iter()
        .enumerate()
        .filter_map(|(index, prevout)| prevout.is_none().then_some(index))
        .collect();
    if let Some(prevout) = prevouts[selected] {
        if prevout.script_pubkey != ScriptBuf::from(&object.address) {
            return Err(ArtifactProgramError::PrevoutMismatch.into());
        }
        if prevout.value < object.required_input_amount {
            return Err(ArtifactProgramError::UnderfundedInput {
                available: prevout.value,
                required: object.required_input_amount,
            }
            .into());
        }
    }
    let input = &psbt.inputs[selected];
    if input
        .witness_script
        .as_ref()
        .is_some_and(|script| Some(script) != expected.witness_script.as_ref())
        || input
            .redeem_script
            .as_ref()
            .is_some_and(|script| Some(script) != expected.redeem_script.as_ref())
    {
        return Err(SpendPlanError::ConflictingScripts);
    }
    if expected.tap_internal_key.is_some()
        && (input
            .tap_internal_key
            .is_some_and(|key| Some(key) != expected.tap_internal_key)
            || input
                .tap_merkle_root
                .is_some_and(|root| Some(root) != expected.tap_merkle_root)
            || input
                .tap_scripts
                .iter()
                .any(|(control, script)| expected.tap_scripts.get(control) != Some(script)))
    {
        return Err(ProgramError::ConflictingTaprootMetadata.into());
    }
    let funding = if missing.contains(&selected) {
        PlanCheck::Unknown
    } else {
        PlanCheck::Met
    };
    Ok((missing, funding))
}

struct Provider<'a> {
    assets: &'a SpendAssets,
    psbt: Option<(&'a Psbt, usize)>,
    program_keys: &'a BTreeMap<(XOnlyPublicKey, ProgramSpendPath), Vec<&'a ProgramRequirement>>,
    hypothetical_assets: bool,
    hypothetical_transaction: bool,
    hypothetical_template: Option<sha256::Hash>,
    actual_template: Option<sha256::Hash>,
    work: &'a PlanningWork,
}

impl<'a> Provider<'a> {
    fn fallback(&self, transaction: bool) -> Self {
        Self {
            hypothetical_assets: true,
            hypothetical_transaction: transaction || self.hypothetical_transaction,
            ..*self
        }
    }
    fn input(&self) -> Option<&Input> {
        self.psbt.map(|(psbt, index)| &psbt.inputs[index])
    }
    fn program(&self, key: XOnlyPublicKey, path: ProgramSpendPath) -> Option<&ProgramRequirement> {
        let programs = self.program_keys.get(&(key, path))?;
        programs
            .iter()
            .copied()
            .find(|requirement| self.capability(requirement).is_some())
            .or_else(|| programs.first().copied())
    }
    fn capability(&self, requirement: &ProgramRequirement) -> Option<&ProgramCapability> {
        self.assets
            .programs
            .iter()
            .find(|capability| &capability.requirement == requirement)
    }
    fn schnorr(&self, key: XOnlyPublicKey, path: ProgramSpendPath) -> Availability {
        if self.input().is_some_and(|input| match path {
            ProgramSpendPath::KeyPath => input.tap_key_sig.is_some(),
            ProgramSpendPath::ScriptPath(leaf) => input.tap_script_sigs.contains_key(&(key, leaf)),
        }) {
            return Availability::PresentUnverified;
        }
        let available = if let Some(requirement) = self.program(key, path) {
            self.capability(requirement).is_some_and(|capability| {
                capability.signer_available && capability.evidence_available
            })
        } else {
            self.assets.schnorr_keys.contains(&key)
        };
        if available {
            Availability::CanProvide
        } else {
            Availability::Missing
        }
    }
    fn ecdsa(&self, key: PublicKey) -> Availability {
        if self
            .input()
            .is_some_and(|input| input.partial_sigs.contains_key(&key))
        {
            Availability::PresentUnverified
        } else if self.assets.ecdsa_keys.contains(&key) {
            Availability::CanProvide
        } else {
            Availability::Missing
        }
    }
    fn preimage(&self, hash: &HashRequirement) -> Availability {
        let valid = self.input().is_some_and(|input| match hash {
            HashRequirement::Sha256(hash) => input
                .sha256_preimages
                .get(hash)
                .is_some_and(|bytes| bytes.len() == 32 && sha256::Hash::hash(bytes) == *hash),
            HashRequirement::Hash256(hash) => input
                .hash256_preimages
                .get(hash)
                .is_some_and(|bytes| bytes.len() == 32 && sha256d::Hash::hash(bytes) == *hash),
            HashRequirement::Ripemd160(hash) => input
                .ripemd160_preimages
                .get(hash)
                .is_some_and(|bytes| bytes.len() == 32 && ripemd160::Hash::hash(bytes) == *hash),
            HashRequirement::Hash160(hash) => input
                .hash160_preimages
                .get(hash)
                .is_some_and(|bytes| bytes.len() == 32 && hash160::Hash::hash(bytes) == *hash),
        });
        if valid {
            Availability::PresentVerified
        } else if self.assets.preimages.contains(hash) {
            Availability::CanProvide
        } else {
            Availability::Missing
        }
    }
    fn annex_size(&self) -> Option<usize> {
        self.input().and_then(|input| {
            sapio_psbt::annex::get(input)
                .ok()
                .flatten()
                .map(<[u8]>::len)
        })
    }
    fn sig_size(&self, key: XOnlyPublicKey, path: ProgramSpendPath) -> usize {
        self.input()
            .and_then(|input| match path {
                ProgramSpendPath::KeyPath => input.tap_key_sig.as_ref(),
                ProgramSpendPath::ScriptPath(leaf) => input.tap_script_sigs.get(&(key, leaf)),
            })
            .map_or(65, |signature| signature.to_vec().len())
    }
}

// Concrete key implementations avoid overlapping Miniscript's blanket
// AssetProvider implementation for arbitrary downstream Satisfier types.
macro_rules! impl_provider {
    ($pk:ident) => {
        impl AssetProvider<$pk> for Provider<'_> {
            fn provider_lookup_ecdsa_sig(&self, key: &$pk) -> bool {
                self.hypothetical_assets || self.ecdsa(key.to_public_key()).available()
            }
            fn provider_lookup_tap_key_spend_sig(&self, key: &$pk) -> Option<usize> {
                (self.hypothetical_assets
                    || self
                        .schnorr(key.to_x_only_pubkey(), ProgramSpendPath::KeyPath)
                        .available())
                .then(|| self.sig_size(key.to_x_only_pubkey(), ProgramSpendPath::KeyPath))
            }
            fn provider_lookup_tap_leaf_script_sig(
                &self,
                key: &$pk,
                leaf: &TapLeafHash,
            ) -> Option<usize> {
                let path = ProgramSpendPath::ScriptPath(*leaf);
                (self.hypothetical_assets || self.schnorr(key.to_x_only_pubkey(), path).available())
                    .then(|| self.sig_size(key.to_x_only_pubkey(), path))
            }
            fn provider_lookup_sha256(&self, hash: &sha256::Hash) -> bool {
                self.hypothetical_assets
                    || self.preimage(&HashRequirement::Sha256(*hash)).available()
            }
            fn provider_lookup_hash256(&self, hash: &miniscript::hash256::Hash) -> bool {
                self.hypothetical_assets
                    || self
                        .preimage(&HashRequirement::Hash256(sha256d::Hash::from_byte_array(
                            hash.to_byte_array(),
                        )))
                        .available()
            }
            fn provider_lookup_ripemd160(&self, hash: &ripemd160::Hash) -> bool {
                self.hypothetical_assets
                    || self
                        .preimage(&HashRequirement::Ripemd160(*hash))
                        .available()
            }
            fn provider_lookup_hash160(&self, hash: &hash160::Hash) -> bool {
                self.hypothetical_assets
                    || self.preimage(&HashRequirement::Hash160(*hash)).available()
            }
            fn check_older(&self, lock: relative::LockTime) -> bool {
                self.hypothetical_transaction
                    || self.psbt.is_some_and(|(psbt, index)| {
                        <PsbtInputSatisfier as Satisfier<$pk>>::check_older(
                            &PsbtInputSatisfier::new(psbt, index),
                            lock,
                        )
                    })
            }
            fn check_after(&self, lock: absolute::LockTime) -> bool {
                self.hypothetical_transaction
                    || self.psbt.is_some_and(|(psbt, index)| {
                        <PsbtInputSatisfier as Satisfier<$pk>>::check_after(
                            &PsbtInputSatisfier::new(psbt, index),
                            lock,
                        )
                    })
            }
            fn check_tx_template(&self, hash: sha256::Hash) -> bool {
                if self.hypothetical_transaction {
                    self.hypothetical_template == Some(hash)
                } else {
                    self.actual_template == Some(hash)
                }
            }
        }
    };
}
impl_provider!(XOnlyPublicKey);
impl_provider!(DefiniteDescriptorKey);

fn unsupported(path: SpendPath, policy: String) -> BranchPlan {
    BranchPlan {
        path,
        policy,
        status: BranchStatus::Unsupported,
        requirements: vec![],
        witness_template: vec![],
        satisfaction_weight_upper_bound: None,
        witness_bytes_upper_bound: None,
        transaction_compatible: PlanCheck::Unknown,
    }
}

fn signature_requirement(
    key: XOnlyPublicKey,
    path: ProgramSpendPath,
    provider: &Provider<'_>,
) -> SpendRequirement {
    let signature = provider.schnorr(key, path);
    if let Some(requirement) = provider.program(key, path) {
        let capability = provider.capability(requirement);
        SpendRequirement::Program {
            requirement: requirement.clone(),
            signature,
            codec: capability.map(|capability| capability.codec.clone()),
            evidence: if capability.is_some_and(|capability| capability.evidence_available) {
                Availability::CanProvide
            } else {
                Availability::Missing
            },
            signer_available: capability.is_some_and(|capability| capability.signer_available),
        }
    } else {
        SpendRequirement::SchnorrSignature {
            key,
            availability: signature,
        }
    }
}

fn key_plan(key: XOnlyPublicKey, provider: &Provider<'_>) -> BranchPlan {
    if let Err(reason) = provider.work.charge(1) {
        return unsupported(SpendPath::KeyPath, reason.into());
    }
    let requirement = signature_requirement(key, ProgramSpendPath::KeyPath, provider);
    let size = provider.sig_size(key, ProgramSpendPath::KeyPath);
    let mut stack = vec![WitnessItem {
        description: format!("Schnorr key-path signature for {key}"),
        serialized_bytes: item_size(size),
    }];
    append_annex(&mut stack, provider);
    let bytes = stack_size(&stack);
    BranchPlan {
        path: SpendPath::KeyPath,
        policy: format!("pk({key})"),
        status: if missing(&requirement) {
            BranchStatus::MissingAssets
        } else {
            BranchStatus::Planned
        },
        requirements: vec![requirement],
        witness_template: stack,
        satisfaction_weight_upper_bound: Some(Weight::from_wu(bytes + 4)),
        witness_bytes_upper_bound: Some(bytes),
        transaction_compatible: PlanCheck::Met,
    }
}

fn tap_plan(
    leaf: TapLeafHash,
    script: &ScriptBuf,
    control: &ControlBlock,
    native: Option<&Miniscript<XOnlyPublicKey, Tap>>,
    provider: &Provider<'_>,
) -> BranchPlan {
    let path = SpendPath::ScriptPath(leaf);
    if let Err(reason) = provider.work.charge(1) {
        return unsupported(path, reason.into());
    }
    let decoded = Miniscript::<XOnlyPublicKey, Tap>::decode_consensus(script).ok();
    let Some(miniscript) = native.or(decoded.as_ref()) else {
        return unsupported(
            path,
            format!("Opaque TapScript: {}", script.to_hex_string()),
        );
    };
    let profile = match NativeProfile::script(miniscript, provider.work) {
        Ok(profile) => profile,
        Err(reason) => return unsupported(path, reason.into()),
    };
    let selected = select_satisfaction(provider, |assets| {
        choose_ctv_candidate(
            &profile,
            assets,
            |assets| {
                let candidate = miniscript.build_template(assets);
                matches!(candidate.stack, Witness::Stack(_)).then_some(candidate)
            },
            |left, right| left.stack < right.stack,
        )
    });
    let (satisfaction, transaction) = match selected {
        Ok(Some(selected)) => selected,
        Ok(None) => return unsupported(path, miniscript.to_string()),
        Err(reason) => return unsupported(path, reason.into()),
    };
    let mut plan = satisfaction_plan(
        path,
        miniscript.to_string(),
        satisfaction.stack,
        satisfaction.absolute_timelock.map(Into::into),
        satisfaction.relative_timelock.map(Into::into),
        satisfaction.tx_template,
        provider,
        transaction,
    );
    if plan.status == BranchStatus::Unsupported {
        return plan;
    }
    let present = provider.input().is_some_and(|input| {
        input.tap_scripts.get(control) == Some(&(script.clone(), LeafVersion::TapScript))
    });
    plan.requirements.push(SpendRequirement::TaprootProof {
        leaf,
        availability: if present {
            Availability::PresentVerified
        } else {
            Availability::CanProvide
        },
    });
    plan.witness_template.push(WitnessItem {
        description: format!("TapScript({})", script.to_hex_string()),
        serialized_bytes: item_size(script.len()),
    });
    plan.witness_template.push(WitnessItem {
        description: format!("ControlBlock({})", control.serialize().as_hex()),
        serialized_bytes: item_size(control.size()),
    });
    append_annex(&mut plan.witness_template, provider);
    let bytes = stack_size(&plan.witness_template);
    plan.witness_bytes_upper_bound = Some(bytes);
    plan.satisfaction_weight_upper_bound = Some(Weight::from_wu(bytes + 4));
    plan
}

fn descriptor_plan(expression: &str, provider: &Provider<'_>) -> BranchPlan {
    let path = SpendPath::Descriptor;
    if let Err(reason) = provider.work.charge(1) {
        return unsupported(path, reason.into());
    }
    let Ok(descriptor) = Descriptor::<DefiniteDescriptorKey>::from_str(expression) else {
        return unsupported(path, expression.into());
    };
    let profile = match NativeProfile::descriptor(&descriptor, provider.work) {
        Ok(profile) => profile,
        Err(reason) => return unsupported(path, reason.into()),
    };
    let selected = select_satisfaction(provider, |assets| {
        choose_ctv_candidate(
            &profile,
            assets,
            |assets| descriptor.clone().plan(assets).ok(),
            |left, right| left.satisfaction_weight() < right.satisfaction_weight(),
        )
    });
    let (native, transaction) = match selected {
        Ok(Some(selected)) => selected,
        Ok(None) => return unsupported(path, expression.into()),
        Err(reason) => return unsupported(path, reason.into()),
    };
    let mut plan = satisfaction_plan(
        SpendPath::Descriptor,
        expression.into(),
        Witness::Stack(native.witness_template().clone()),
        native.absolute_timelock,
        native.relative_timelock,
        native.tx_template,
        provider,
        transaction,
    );
    if plan.status == BranchStatus::Unsupported {
        return plan;
    }
    let witness = native.witness_version().is_some();
    // The pinned Plan contains the satisfaction stack but omits WSH/P2SH's
    // script. Build its enclosing encoding here, including legacy PUSHDATA
    // and the scriptSig CompactSize boundary, rather than treating a scriptSig
    // as a serialized witness stack. ECDSA placeholders reserve 73 bytes.
    if !witness {
        for (item, placeholder) in plan
            .witness_template
            .iter_mut()
            .zip(native.witness_template())
        {
            item.serialized_bytes = script_push_size(placeholder_data_size(placeholder));
        }
    }
    let script = match &native.descriptor {
        Descriptor::Wsh(wsh) => Some(wsh.inner_script()),
        Descriptor::Sh(sh) => match sh.as_inner() {
            miniscript::descriptor::ShInner::Wsh(wsh) => Some(wsh.inner_script()),
            miniscript::descriptor::ShInner::Wpkh(_) => None,
            _ => Some(sh.inner_script()),
        },
        _ => None,
    };
    if let Some(script) = script {
        plan.witness_template.push(WitnessItem {
            description: format!(
                "{}({})",
                if witness {
                    "WitnessScript"
                } else {
                    "RedeemScript"
                },
                script.to_hex_string()
            ),
            serialized_bytes: if witness {
                item_size(script.len())
            } else {
                script_push_size(script.len())
            },
        });
    }
    let (witness_bytes, script_sig_bytes) = if witness {
        (
            stack_size(&plan.witness_template),
            item_size(native.descriptor.unsigned_script_sig().len()),
        )
    } else {
        let bytes = plan
            .witness_template
            .iter()
            .map(|item| item.serialized_bytes)
            .sum::<u64>();
        (0, bytes + bitcoin::VarInt(bytes).size() as u64)
    };
    plan.witness_bytes_upper_bound = Some(witness_bytes);
    plan.satisfaction_weight_upper_bound =
        Some(Weight::from_wu(witness_bytes + script_sig_bytes * 4));
    plan
}

struct NativeProfile {
    nodes: usize,
    commitments: BTreeSet<sha256::Hash>,
}

impl NativeProfile {
    fn script<Pk: ToPublicKey, Ctx: miniscript::ScriptContext>(
        script: &Miniscript<Pk, Ctx>,
        work: &PlanningWork,
    ) -> Result<Self, &'static str> {
        if script.has_mixed_timelocks() || !script.within_resource_limits() {
            return Err(
                "Native policy has mixed timelocks or exceeds satisfaction resource limits",
            );
        }
        let mut profile = Self {
            nodes: 0,
            commitments: BTreeSet::new(),
        };
        for node in script.iter() {
            work.charge(1)?;
            profile.nodes += 1;
            if let miniscript::Terminal::TxTemplate(hash) = node.node {
                profile.commitments.insert(hash);
            }
        }
        Ok(profile)
    }

    fn descriptor(
        descriptor: &Descriptor<DefiniteDescriptorKey>,
        work: &PlanningWork,
    ) -> Result<Self, &'static str> {
        use miniscript::descriptor::{ShInner, WshInner};
        let check_wsh =
            |wsh: &miniscript::descriptor::Wsh<DefiniteDescriptorKey>| match wsh.as_inner() {
                WshInner::Ms(script) => Some(Self::script(script, work)),
                WshInner::SortedMulti(_) => None,
            };
        let native = match descriptor {
            Descriptor::Bare(bare) => Some(Self::script(bare.as_inner(), work)),
            Descriptor::Wsh(wsh) => check_wsh(wsh),
            Descriptor::Sh(sh) => match sh.as_inner() {
                ShInner::Ms(script) => Some(Self::script(script, work)),
                ShInner::Wsh(wsh) => check_wsh(wsh),
                ShInner::Wpkh(_) | ShInner::SortedMulti(_) => None,
            },
            Descriptor::Pkh(_) | Descriptor::Wpkh(_) | Descriptor::Tr(_) => None,
        };
        if let Some(native) = native {
            return native;
        }
        let mut nodes = 1;
        work.charge(1)?;
        for _ in descriptor.iter_pk() {
            work.charge(1)?;
            nodes += 1;
        }
        Ok(Self {
            nodes,
            commitments: BTreeSet::new(),
        })
    }
}

// Candidates remain internal: every output location still has exactly one
// deterministic selected plan, so SpendPath remains an unambiguous selector.
fn choose_ctv_candidate<T>(
    profile: &NativeProfile,
    provider: &Provider<'_>,
    mut build: impl FnMut(&Provider<'_>) -> Option<T>,
    cheaper: impl Fn(&T, &T) -> bool,
) -> Result<Option<T>, &'static str> {
    if !provider.hypothetical_transaction {
        provider.work.charge(profile.nodes)?;
        return Ok(build(provider));
    }
    let mut best = None;
    // CTV-free candidates win equal-cost ties, then bytewise commitment order.
    for template in std::iter::once(None).chain(profile.commitments.iter().copied().map(Some)) {
        provider.work.charge(profile.nodes)?;
        let assets = Provider {
            hypothetical_template: template,
            ..*provider
        };
        if let Some(candidate) = build(&assets) {
            if best
                .as_ref()
                .is_none_or(|previous| cheaper(&candidate, previous))
            {
                best = Some(candidate);
            }
        }
    }
    Ok(best)
}

fn select_satisfaction<T>(
    provider: &Provider<'_>,
    mut attempt: impl FnMut(&Provider<'_>) -> Result<Option<T>, &'static str>,
) -> Result<Option<(T, PlanCheck)>, &'static str> {
    let compatible = if provider.psbt.is_some() {
        PlanCheck::Met
    } else {
        PlanCheck::Unknown
    };
    if let Some(plan) = attempt(provider)? {
        return Ok(Some((plan, compatible)));
    }
    if let Some(plan) = attempt(&provider.fallback(false))? {
        return Ok(Some((plan, compatible)));
    }
    if provider.psbt.is_some() {
        if let Some(plan) = attempt(&provider.fallback(true))? {
            return Ok(Some((plan, PlanCheck::Unmet)));
        }
    }
    Ok(None)
}

fn satisfaction_plan<Pk: ToPublicKey>(
    path: SpendPath,
    policy: String,
    stack: Witness<Placeholder<Pk>>,
    absolute: Option<absolute::LockTime>,
    relative: Option<relative::LockTime>,
    tx_template: Option<sha256::Hash>,
    provider: &Provider<'_>,
    transaction: PlanCheck,
) -> BranchPlan {
    let Witness::Stack(stack) = stack else {
        return unsupported(path, policy);
    };
    let mut plan = unsupported(path, policy);
    plan.transaction_compatible = transaction;
    for item in &stack {
        let requirement = match item {
            Placeholder::SchnorrSigPk(
                key,
                miniscript::miniscript::satisfy::SchnorrSigType::ScriptSpend { leaf_hash },
                _,
            ) => Some(signature_requirement(
                key.to_x_only_pubkey(),
                ProgramSpendPath::ScriptPath(*leaf_hash),
                provider,
            )),
            Placeholder::SchnorrSigPk(key, _, _) => Some(signature_requirement(
                key.to_x_only_pubkey(),
                ProgramSpendPath::KeyPath,
                provider,
            )),
            Placeholder::EcdsaSigPk(key) => Some(SpendRequirement::EcdsaSignature {
                key: key.to_public_key(),
                availability: provider.ecdsa(key.to_public_key()),
            }),
            Placeholder::Sha256Preimage(hash) => Some(hash_requirement(
                HashRequirement::Sha256(Pk::to_sha256(hash)),
                provider,
            )),
            Placeholder::Hash256Preimage(hash) => Some(hash_requirement(
                HashRequirement::Hash256(sha256d::Hash::from_byte_array(
                    Pk::to_hash256(hash).to_byte_array(),
                )),
                provider,
            )),
            Placeholder::Ripemd160Preimage(hash) => Some(hash_requirement(
                HashRequirement::Ripemd160(Pk::to_ripemd160(hash)),
                provider,
            )),
            Placeholder::Hash160Preimage(hash) => Some(hash_requirement(
                HashRequirement::Hash160(Pk::to_hash160(hash)),
                provider,
            )),
            Placeholder::PubkeyHash(_, _)
            | Placeholder::EcdsaSigPkHash(_)
            | Placeholder::SchnorrSigPkHash(_, _, _) => {
                return unsupported(
                    path,
                    "Unresolved hashed public key in native satisfaction".into(),
                )
            }
            _ => None,
        };
        if let Some(requirement) = requirement {
            if !plan.requirements.contains(&requirement) {
                plan.requirements.push(requirement);
            }
        }
        plan.witness_template.push(WitnessItem {
            description: item.to_string(),
            serialized_bytes: placeholder_size(item),
        });
    }
    if let Some(hash) = tx_template {
        plan.requirements
            .push(SpendRequirement::NativeTemplateHash {
                hash,
                transaction: provider
                    .actual_template
                    .map_or(PlanCheck::Unknown, |actual| check(actual == hash)),
            });
    }
    if let Some(lock) = absolute {
        plan.requirements.push(SpendRequirement::AbsoluteTimelock {
            value: lock.to_consensus_u32(),
            transaction: provider.psbt.map_or(PlanCheck::Unknown, |(psbt, index)| {
                check(<PsbtInputSatisfier as Satisfier<Pk>>::check_after(
                    &PsbtInputSatisfier::new(psbt, index),
                    lock,
                ))
            }),
            chain_maturity: PlanCheck::Unknown,
        });
    }
    if let Some(lock) = relative {
        plan.requirements.push(SpendRequirement::RelativeTimelock {
            value: lock.to_consensus_u32(),
            transaction: provider.psbt.map_or(PlanCheck::Unknown, |(psbt, index)| {
                check(<PsbtInputSatisfier as Satisfier<Pk>>::check_older(
                    &PsbtInputSatisfier::new(psbt, index),
                    lock,
                ))
            }),
            chain_maturity: PlanCheck::Unknown,
        });
    }
    plan.status = if transaction == PlanCheck::Unmet {
        BranchStatus::IncompatibleTransaction
    } else if plan.requirements.iter().any(missing) {
        BranchStatus::MissingAssets
    } else {
        BranchStatus::Planned
    };
    plan
}

fn hash_requirement(hash: HashRequirement, provider: &Provider<'_>) -> SpendRequirement {
    let availability = provider.preimage(&hash);
    SpendRequirement::Preimage { hash, availability }
}
fn check(value: bool) -> PlanCheck {
    if value {
        PlanCheck::Met
    } else {
        PlanCheck::Unmet
    }
}
fn missing(requirement: &SpendRequirement) -> bool {
    match requirement {
        SpendRequirement::SchnorrSignature { availability, .. }
        | SpendRequirement::EcdsaSignature { availability, .. }
        | SpendRequirement::Preimage { availability, .. } => !availability.available(),
        SpendRequirement::Program { signature, .. } => !signature.available(),
        _ => false,
    }
}
fn item_size(size: usize) -> u64 {
    (bitcoin::VarInt(size as u64).size() + size) as u64
}
fn stack_size(stack: &[WitnessItem]) -> u64 {
    bitcoin::VarInt(stack.len() as u64).size() as u64
        + stack.iter().map(|item| item.serialized_bytes).sum::<u64>()
}
fn append_annex(stack: &mut Vec<WitnessItem>, provider: &Provider<'_>) {
    if let Some(size) = provider.annex_size() {
        stack.push(WitnessItem {
            description: "PSBT annex (contents omitted)".into(),
            serialized_bytes: item_size(size),
        });
    }
}
fn placeholder_size<Pk: ToPublicKey>(item: &Placeholder<Pk>) -> u64 {
    match item {
        Placeholder::Pubkey(_, size) | Placeholder::PubkeyHash(_, size) => *size as u64,
        Placeholder::EcdsaSigPk(_) | Placeholder::EcdsaSigPkHash(_) => item_size(73),
        Placeholder::SchnorrSigPk(_, _, size) | Placeholder::SchnorrSigPkHash(_, _, size) => {
            item_size(*size)
        }
        Placeholder::HashDissatisfaction
        | Placeholder::Sha256Preimage(_)
        | Placeholder::Hash256Preimage(_)
        | Placeholder::Ripemd160Preimage(_)
        | Placeholder::Hash160Preimage(_) => item_size(32),
        Placeholder::PushOne => item_size(1),
        Placeholder::PushZero => item_size(0),
        Placeholder::TapScript(script) => item_size(script.len()),
        Placeholder::TapControlBlock(control) => item_size(control.size()),
    }
}

fn placeholder_data_size<Pk: ToPublicKey>(item: &Placeholder<Pk>) -> usize {
    match item {
        Placeholder::Pubkey(_, size) | Placeholder::PubkeyHash(_, size) => size - 1,
        Placeholder::EcdsaSigPk(_) | Placeholder::EcdsaSigPkHash(_) => 73,
        Placeholder::SchnorrSigPk(_, _, size) | Placeholder::SchnorrSigPkHash(_, _, size) => *size,
        Placeholder::HashDissatisfaction
        | Placeholder::Sha256Preimage(_)
        | Placeholder::Hash256Preimage(_)
        | Placeholder::Ripemd160Preimage(_)
        | Placeholder::Hash160Preimage(_) => 32,
        Placeholder::PushOne => 1,
        Placeholder::PushZero => 0,
        Placeholder::TapScript(script) => script.len(),
        Placeholder::TapControlBlock(control) => control.size(),
    }
}

fn script_push_size(bytes: usize) -> u64 {
    let prefix = match bytes {
        0..=75 => 1,
        76..=255 => 2,
        256..=65_535 => 3,
        _ => 5,
    };
    (bytes + prefix) as u64
}

#[cfg(test)]
mod tests;
