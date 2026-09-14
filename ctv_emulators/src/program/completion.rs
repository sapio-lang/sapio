//! Execute an explicitly selected spend from portable public inputs.
//!
//! The artifact and saved intent are caller-selected inputs, not authenticated
//! deployment policy. Every operation revalidates their relationship. Original
//! program requests remain immutable while the current PSBT accumulates assets.

use super::spend_plan::{
    validate_spend_selection, Availability, BranchPlan, BranchStatus, PreparedSpend, SpendPath,
    SpendPlanError, SpendRequirement,
};
use super::{
    prepare_program_request, put_signature, target_signature, validate_program_response,
    ArtifactProgramError, ProgramError, ProgramSigningRequest, ProgramSpendPath, PSBT,
};
use bitcoin::psbt::{Input, Psbt};
use bitcoin::secp256k1::{Secp256k1, Signing, Verification};
use sapio::contract::abi::object::{Object, ProgramRequirement};
use sapio_base::CTVHash;
use sapio_psbt::selected::{SatisfactionRecipe, SelectedError, SignatureSlot};
use sapio_psbt::SigningKey;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// A portable selected witness and its immutable original signing requests.
///
/// Request indices are stable within this value. Keep this intent unchanged and
/// save the current PSBT separately when collecting signatures across processes.
/// Deserialization grants no authority: methods check the trusted artifact,
/// canonical witness selection, request baselines and current transaction.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpendIntent {
    input_index: usize,
    path: SpendPath,
    recipe: SatisfactionRecipe,
    baseline: PSBT,
    program_requests: Vec<ProgramSigningRequest>,
}

/// A selected spend cannot accept a contribution or complete its witness.
#[derive(Debug)]
pub enum SpendCompletionError {
    /// The artifact, selection or supplied funding is invalid.
    Selection(SpendPlanError),
    /// The stored request does not describe this preparation.
    InvalidIntent(&'static str),
    /// The current PSBT changed fixed data or discarded a supplied asset.
    ChangedPSBT(&'static str),
    /// No original request exists at this index.
    RequestIndex(usize),
    /// A response violates the evaluated signing protocol.
    Program(ProgramError),
    /// A selected signature/preimage is missing or invalid.
    Selected(SelectedError),
    /// Required assets are still missing; the original PSBT is preserved.
    Incomplete(Box<BranchPlan>),
    /// Actual finalized fees do not meet the retained local requirements.
    Funding(sapio::template::funding::FundingError),
    /// The complete transaction has invalid aggregate input/output amounts.
    Balance(bitcoin::psbt::Error),
    /// Finalization did not establish the weight needed by local fee policy.
    FeeRatePending,
}

impl fmt::Display for SpendCompletionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Selection(error) => error.fmt(f),
            Self::InvalidIntent(reason) => write!(f, "invalid spend intent: {reason}"),
            Self::ChangedPSBT(reason) => {
                write!(f, "current PSBT differs from preparation: {reason}")
            }
            Self::RequestIndex(index) => write!(f, "program request {index} does not exist"),
            Self::Program(error) => error.fmt(f),
            Self::Selected(error) => error.fmt(f),
            Self::Incomplete(plan) => {
                write!(f, "selected spend is incomplete: {:?}", plan.requirements)
            }
            Self::Funding(error) => error.fmt(f),
            Self::Balance(error) => error.fmt(f),
            Self::FeeRatePending => f.write_str(
                "final transaction weight is still required to check the retained fee rate",
            ),
        }
    }
}

impl std::error::Error for SpendCompletionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Selection(error) => Some(error),
            Self::Program(error) => Some(error),
            Self::Selected(error) => Some(error),
            Self::Funding(error) => Some(error),
            Self::Balance(error) => Some(error),
            _ => None,
        }
    }
}

impl From<SpendPlanError> for SpendCompletionError {
    fn from(error: SpendPlanError) -> Self {
        Self::Selection(error)
    }
}
impl From<ArtifactProgramError> for SpendCompletionError {
    fn from(error: ArtifactProgramError) -> Self {
        Self::Selection(error.into())
    }
}
impl From<ProgramError> for SpendCompletionError {
    fn from(error: ProgramError) -> Self {
        Self::Program(error)
    }
}
impl From<SelectedError> for SpendCompletionError {
    fn from(error: SelectedError) -> Self {
        Self::Selected(error)
    }
}

impl SpendIntent {
    /// Retain the exact selected witness and original request baselines.
    ///
    /// Key-path requests omit unrelated leaf proofs, key origins and internal
    /// key annotations. The full PSBT retains them for local completion; the
    /// program protocol authenticates its own key using the output commitment.
    pub fn from_prepared(prepared: PreparedSpend) -> Self {
        let mut requests = prepared.program_requests;
        for request in &mut requests {
            project_request(request);
        }
        Self {
            input_index: prepared.input_index,
            path: prepared.plan.path,
            recipe: prepared.recipe,
            baseline: PSBT(prepared.psbt),
            program_requests: requests,
        }
    }

    /// Initial PSBT, before collecting additional signatures or preimages.
    pub fn baseline_psbt(&self) -> &Psbt {
        &self.baseline.0
    }

    /// Stable original requests, indexed in selected witness order.
    ///
    /// Validate a deserialized intent with `status` before exporting requests to
    /// an explicitly configured signer. Responses refer to these exact baselines.
    pub fn requests(&self) -> &[ProgramSigningRequest] {
        &self.program_requests
    }

    /// Exact program identities and oracle roots for the indexed requests.
    ///
    /// Validate against the caller's artifact before choosing an explicit
    /// signer. Identical program instances may require different public roots.
    pub fn request_requirements(
        &self,
        object: &Object,
    ) -> Result<Vec<ProgramRequirement>, SpendCompletionError> {
        self.validate(object, self.baseline_psbt())
            .map(|(_, _, requirements)| requirements)
    }

    /// Report current requirements for exactly the retained witness.
    pub fn status(
        &self,
        object: &Object,
        current: &Psbt,
    ) -> Result<BranchPlan, SpendCompletionError> {
        self.validate(object, current).map(|(plan, _, _)| plan)
    }

    /// Sign only ordinary native slots selected by the validated artifact.
    ///
    /// Program slots are never supplied to the native key bag. Signing sponsors
    /// or other contract inputs remains a separate explicit operation.
    pub fn sign_native<C: Signing + Verification>(
        &self,
        object: &Object,
        current: &mut Psbt,
        keys: &SigningKey,
        secp: &Secp256k1<C>,
    ) -> Result<(), SpendCompletionError> {
        let (_, native, _) = self.validate(object, current)?;
        keys.sign_selected_input_mut(current, secp, self.input_index, &native)?;
        Ok(())
    }

    /// Import one authenticated program signature without replacing the PSBT.
    ///
    /// Responses are checked against their original request, so they can arrive
    /// in any order. An identical repeated response is harmless; conflicting
    /// signatures and unrelated response changes leave current data unchanged.
    pub fn merge_response(
        &self,
        object: &Object,
        current: &mut Psbt,
        request_index: usize,
        response: &Psbt,
    ) -> Result<(), SpendCompletionError> {
        let (_, _, requirements) = self.validate(object, current)?;
        let request = self
            .program_requests
            .get(request_index)
            .ok_or(SpendCompletionError::RequestIndex(request_index))?;
        let requirement = &requirements[request_index];
        validate_program_response(request, response, requirement.program.root())?;
        let key = requirement
            .program
            .derive_public_key()
            .map_err(ProgramError::Instance)?;
        let signature = *target_signature(response, request, key).ok_or(
            ProgramError::InvalidResponse("requested signature is missing"),
        )?;
        if let Some(existing) = target_signature(current, request, key) {
            if *existing != signature {
                return Err(ProgramError::ConflictingSignature.into());
            }
            return Ok(());
        }
        put_signature(current, request.input_index, request.path, key, signature);
        Ok(())
    }

    /// Complete the selected witness, verify every input, and check final fees.
    ///
    /// Unselected inputs may finalize using their already-present assets. The
    /// returned PSBT is ready for normal extraction. Missing or invalid assets
    /// leave the caller's partially signed PSBT unchanged.
    pub fn finalize<C: Verification>(
        &self,
        object: &Object,
        current: &Psbt,
        secp: &Secp256k1<C>,
    ) -> Result<Psbt, SpendCompletionError> {
        let (plan, _, _) = self.validate(object, current)?;
        if plan.status == BranchStatus::MissingAssets {
            return Err(SpendCompletionError::Incomplete(Box::new(plan)));
        }
        let mut finalized = current.clone();
        sapio_psbt::selected::finalize_selected(
            &mut finalized,
            secp,
            &BTreeMap::from([(self.input_index, self.recipe.clone())]),
        )?;
        finalized.fee().map_err(SpendCompletionError::Balance)?;
        // The catalog lookup is construction identity (unsigned input zero).
        // Exact native CTV enforcement has already checked final scriptSigs.
        if self.input_index == 0 {
            let hash = self.baseline.0.unsigned_tx.get_ctv_hash(0);
            if let Some(template) = object
                .ctv_to_tx
                .get(&hash)
                .or_else(|| object.suggested_txs.get(&hash))
            {
                let funding = template
                    .check_funded_psbt(&finalized)
                    .map_err(SpendCompletionError::Funding)?;
                if funding.fee_rate_pending {
                    return Err(SpendCompletionError::FeeRatePending);
                }
            }
        }
        Ok(finalized)
    }

    fn validate(
        &self,
        object: &Object,
        current: &Psbt,
    ) -> Result<(BranchPlan, Vec<SignatureSlot>, Vec<ProgramRequirement>), SpendCompletionError>
    {
        let (baseline_plan, _) = validate_spend_selection(
            object,
            self.path,
            &self.baseline.0,
            self.input_index,
            &self.recipe,
        )?;
        let mut unsigned = Vec::new();
        for requirement in &baseline_plan.requirements {
            if let SpendRequirement::Program {
                requirement,
                signature,
                ..
            } = requirement
            {
                if *signature == Availability::PresentUnverified {
                    // Validate a signature supplied before preparation as well.
                    prepare_program_request(
                        object,
                        requirement,
                        self.baseline.0.clone(),
                        self.input_index as u32,
                        Vec::new(),
                    )?;
                } else {
                    unsigned.push(requirement.clone());
                }
            }
        }
        if unsigned.len() != self.program_requests.len() {
            return Err(SpendCompletionError::InvalidIntent(
                "original requests do not match the selected unsigned program slots",
            ));
        }
        for (requirement, request) in unsigned.iter().zip(&self.program_requests) {
            let mut expected = prepare_program_request(
                object,
                requirement,
                self.baseline.0.clone(),
                self.input_index as u32,
                request.witness.clone(),
            )?;
            project_request(&mut expected);
            if *request != expected {
                return Err(SpendCompletionError::InvalidIntent(
                    "program request identity, location or baseline was changed",
                ));
            }
        }
        check_progress(&self.baseline.0, current, self.input_index)?;
        let (plan, native) =
            validate_spend_selection(object, self.path, current, self.input_index, &self.recipe)?;
        Ok((plan, native, unsigned))
    }
}

fn project_request(request: &mut ProgramSigningRequest) {
    if request.path == ProgramSpendPath::KeyPath {
        if let Some(input) = request.psbt.0.inputs.get_mut(request.input_index as usize) {
            input.tap_scripts.clear();
            input.tap_key_origins.clear();
            input.tap_internal_key = None;
        }
    }
}

fn contains<K: Ord, V: PartialEq>(original: &BTreeMap<K, V>, current: &BTreeMap<K, V>) -> bool {
    original
        .iter()
        .all(|(key, value)| current.get(key) == Some(value))
}

fn strip_assets(input: &mut Input) {
    input.partial_sigs.clear();
    input.tap_key_sig = None;
    input.tap_script_sigs.clear();
    input.ripemd160_preimages.clear();
    input.sha256_preimages.clear();
    input.hash160_preimages.clear();
    input.hash256_preimages.clear();
    input.final_script_sig = None;
    input.final_script_witness = None;
}

fn check_progress(
    baseline: &Psbt,
    current: &Psbt,
    selected: usize,
) -> Result<(), SpendCompletionError> {
    sapio_psbt::validate_psbt(current).map_err(ProgramError::Psbt)?;
    if baseline.inputs.len() != current.inputs.len() {
        return Err(SpendCompletionError::ChangedPSBT("input count changed"));
    }
    for (index, (before, after)) in baseline.inputs.iter().zip(&current.inputs).enumerate() {
        if !contains(&before.partial_sigs, &after.partial_sigs)
            || before
                .tap_key_sig
                .is_some_and(|sig| after.tap_key_sig != Some(sig))
            || !contains(&before.tap_script_sigs, &after.tap_script_sigs)
            || !contains(&before.ripemd160_preimages, &after.ripemd160_preimages)
            || !contains(&before.sha256_preimages, &after.sha256_preimages)
            || !contains(&before.hash160_preimages, &after.hash160_preimages)
            || !contains(&before.hash256_preimages, &after.hash256_preimages)
            || before
                .final_script_sig
                .as_ref()
                .is_some_and(|value| after.final_script_sig.as_ref() != Some(value))
            || before
                .final_script_witness
                .as_ref()
                .is_some_and(|value| after.final_script_witness.as_ref() != Some(value))
        {
            return Err(SpendCompletionError::ChangedPSBT(
                "an existing asset was removed or replaced",
            ));
        }
        if index == selected
            && (after.final_script_sig.is_some() || after.final_script_witness.is_some())
        {
            return Err(SpendCompletionError::ChangedPSBT(
                "selected input is already finalized",
            ));
        }
    }
    let mut before = baseline.clone();
    let mut after = current.clone();
    for input in &mut before.inputs {
        strip_assets(input);
    }
    for input in &mut after.inputs {
        strip_assets(input);
    }
    if before != after {
        return Err(SpendCompletionError::ChangedPSBT(
            "transaction, funding, scripts, sighash, annex or other fixed metadata changed",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
