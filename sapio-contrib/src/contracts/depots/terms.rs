//! Public depot parameters, the byte formats shared with the depots
//! evaluator, and the canonical resolution templates.

use super::{ClaimOutput, Depot, HitClaimOutput, HitInsuranceOutput, TargetOutput};
use bitcoin::bip32::Xpub;
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::{sha256, Hash};
use bitcoin::{Amount, XOnlyPublicKey};
use sapio::contract::{CompilationError, Compiled, Context};
use sapio::template::{OutputAmount, Surplus, Template};
use sapio_base::fragments::template_hash;
use sapio_base::program::{EmulatedProgram, ProgramInstance};
use sapio_base::timelocks::{AbsHeight, RelHeight};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;
use std::sync::Arc;

/// The compiled depots evaluator. Its exact bytes are part of every depot
/// program's identity: rebuilding different bytes changes every depot address.
pub const DEPOTS_WASM: &[u8] = include_bytes!("../../../../evaluators/artifacts/depots.wasm");
const _: () = assert!(DEPOTS_WASM.len() <= 65_536);

/// Evaluator parameter length: stage, mode, depot id, two keys, prime, hash.
pub const PARAMETERS_LENGTH: usize = 138;
/// Witness section proving an operator-selected target: `t_le || TaSig`.
pub const TARGET_SECTION_LENGTH: usize = 8 + 64;
/// One guess tuple: `index_le || salt || guess_le || GuSig || clm || clm'`.
pub const TUPLE_LENGTH: usize = 8 + 32 + 8 + 64 + 32 + 32;

pub(super) fn invalid(error: impl std::fmt::Display) -> CompilationError {
    CompilationError::Custom(error.to_string().into())
}

/// Which resolution predicate a program leaf enforces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Funding -> Targ: the operator reveals the target.
    TargetReveal = 0,
    /// Targ -> Hit: a hit owner forces the fixed 50/50 split.
    Hit = 1,
    /// Hit output 1 -> Clm: bind the exact blind-signed claim transaction.
    Claim = 2,
    /// Two distinct guess signatures for the target burn the output.
    Burn = 3,
}

/// Whether a resolution branch uses an operator-selected or default target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ResolutionMode {
    /// The operator proves a target `t` with the target signature TaSig.
    Operator,
    /// The paper's primed transactions: the target is fixed at zero, so no
    /// operator signature is needed and any user can drive resolution.
    Default,
}

impl ResolutionMode {
    fn byte(self) -> u8 {
        match self {
            ResolutionMode::Operator => 0,
            ResolutionMode::Default => 1,
        }
    }
}

fn tagged(tag: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let tag_hash = sha256::Hash::hash(tag);
    let mut engine = sha256::Hash::engine();
    use bitcoin::hashes::HashEngine;
    engine.input(tag_hash.as_ref());
    engine.input(tag_hash.as_ref());
    for part in parts {
        engine.input(part);
    }
    sha256::Hash::from_engine(engine).to_byte_array()
}

/// Immutable public depot parameters.
///
/// The depot's on-chain footprint is independent of its channels: channels
/// exist only as operator blind signatures over witness evidence, so a depot
/// address commits to no user set and guesses stay secret until claim time.
#[derive(Clone, Debug)]
pub struct DepotTerms {
    operator: XOnlyPublicKey,
    target_key: XOnlyPublicKey,
    guess_key: XOnlyPublicKey,
    oracle_root: Xpub,
    value: Amount,
    prime: u64,
    expiry: u32,
    to_self_delay: u16,
    maximum_sponsor_fee: Amount,
    depot_id: [u8; 32],
}

impl DepotTerms {
    /// Fix the depot's keys, value, randomization prime and time windows.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        operator: XOnlyPublicKey,
        target_key: XOnlyPublicKey,
        guess_key: XOnlyPublicKey,
        oracle_root: Xpub,
        value: Amount,
        prime: u64,
        expiry: AbsHeight,
        to_self_delay: u16,
        maximum_sponsor_fee: Amount,
    ) -> Result<Self, CompilationError> {
        if value == Amount::ZERO || !value.to_sat().is_multiple_of(2) {
            return Err(invalid(
                "depot value must be positive and even so the Hit transaction splits exactly",
            ));
        }
        if prime < 2 {
            return Err(invalid("depot prime must be at least 2"));
        }
        if to_self_delay == 0 || to_self_delay.checked_mul(3).is_none() {
            return Err(invalid(
                "to_self_delay must be positive and 3*to_self_delay must fit a relative height",
            ));
        }
        if maximum_sponsor_fee == Amount::ZERO {
            return Err(invalid("fee sponsor ceiling must be positive"));
        }
        let default_height = expiry
            .get()
            .checked_add(u32::from(to_self_delay))
            .ok_or_else(|| invalid("depot expiry overflows"))?;
        AbsHeight::try_from(default_height)?;
        let depot_id = tagged(
            b"depots/v1/id",
            &[
                &operator.serialize(),
                &target_key.serialize(),
                &guess_key.serialize(),
                &oracle_root.encode(),
                &value.to_sat().to_le_bytes(),
                &prime.to_le_bytes(),
                &expiry.get().to_le_bytes(),
                &u32::from(to_self_delay).to_le_bytes(),
            ],
        );
        Ok(Self {
            operator,
            target_key,
            guess_key,
            oracle_root,
            value,
            prime,
            expiry: expiry.get(),
            to_self_delay,
            maximum_sponsor_fee,
            depot_id,
        })
    }

    /// The operator's payout key for sweeps and channel settlement.
    pub fn operator(&self) -> XOnlyPublicKey {
        self.operator
    }
    /// The key whose signature reveals the depot's target.
    pub fn target_key(&self) -> XOnlyPublicKey {
        self.target_key
    }
    /// The key whose blind signatures commit channels at purchase time.
    pub fn guess_key(&self) -> XOnlyPublicKey {
        self.guess_key
    }
    /// The emulation oracle every program leaf derives its key from.
    pub fn oracle_root(&self) -> &Xpub {
        &self.oracle_root
    }
    /// The depot value `D`; a hit funds a channel worth `D/2` on-chain.
    pub fn value(&self) -> Amount {
        self.value
    }
    /// The randomization prime `P`; a channel hits with probability `1/P`.
    pub fn prime(&self) -> u64 {
        self.prime
    }
    /// The depot expiry height `E`.
    pub fn expiry(&self) -> Result<AbsHeight, CompilationError> {
        Ok(AbsHeight::try_from(self.expiry)?)
    }
    /// The depot expiry height `E` as a raw height.
    pub fn expiry_height(&self) -> u32 {
        self.expiry
    }
    /// The `tsd` monitoring window, in blocks.
    pub fn to_self_delay(&self) -> u16 {
        self.to_self_delay
    }
    /// Local ceiling for sponsor value consumed as fees; not a Script rule.
    pub fn maximum_sponsor_fee(&self) -> Amount {
        self.maximum_sponsor_fee
    }
    /// The commitment binding every depot signature to these exact terms.
    pub fn depot_id(&self) -> &[u8; 32] {
        &self.depot_id
    }
    /// The height at which the default target transaction becomes valid.
    pub fn default_height(&self) -> Result<AbsHeight, CompilationError> {
        Ok(AbsHeight::try_from(
            self.expiry + u32::from(self.to_self_delay),
        )?)
    }

    /// The depot funding contract.
    pub fn funding(&self) -> Depot {
        Depot {
            terms: self.clone(),
        }
    }

    /// The Target transaction's output for one resolution branch.
    pub fn target_output(&self, mode: ResolutionMode) -> TargetOutput {
        TargetOutput {
            terms: self.clone(),
            mode,
        }
    }

    /// Hit output 0: the insurance half for one resolution branch.
    pub fn hit_insurance_output(&self, mode: ResolutionMode) -> HitInsuranceOutput {
        HitInsuranceOutput {
            terms: self.clone(),
            mode,
        }
    }

    /// Hit output 1: the claim half for one resolution branch.
    pub fn hit_claim_output(&self, mode: ResolutionMode) -> HitClaimOutput {
        HitClaimOutput {
            terms: self.clone(),
            mode,
        }
    }

    /// The settlement layer a specific user's claim transaction funds.
    pub fn claim_output(&self, user: XOnlyPublicKey, mode: ResolutionMode) -> ClaimOutput {
        ClaimOutput {
            terms: self.clone(),
            user,
            mode,
        }
    }

    /// One depots-evaluator program leaf for this depot.
    pub fn program(
        &self,
        stage: Stage,
        mode: ResolutionMode,
        expected_template_hash: Sha256,
    ) -> Result<EmulatedProgram, CompilationError> {
        let mut parameters = Vec::with_capacity(PARAMETERS_LENGTH);
        parameters.push(stage as u8);
        parameters.push(mode.byte());
        parameters.extend_from_slice(&self.depot_id);
        parameters.extend_from_slice(&self.target_key.serialize());
        parameters.extend_from_slice(&self.guess_key.serialize());
        parameters.extend_from_slice(&self.prime.to_le_bytes());
        parameters.extend_from_slice(&expected_template_hash.to_byte_array());
        debug_assert_eq!(parameters.len(), PARAMETERS_LENGTH);
        let instance =
            ProgramInstance::wasm_v2(DEPOTS_WASM.to_vec(), parameters).map_err(invalid)?;
        EmulatedProgram::new(instance, self.oracle_root).map_err(invalid)
    }

    /// The message the operator's target signature TaSig commits to.
    pub fn target_message(&self, target: u64) -> [u8; 32] {
        tagged(
            b"depots/v1/target",
            &[&self.depot_id, &target.to_le_bytes()],
        )
    }

    /// The message the operator blind-signs when selling one channel.
    pub fn guess_message(
        &self,
        clm: [u8; 32],
        clm_prime: [u8; 32],
        index: u64,
        salt: [u8; 32],
        guess: u64,
    ) -> [u8; 32] {
        tagged(
            b"depots/v1/guess",
            &[
                &self.depot_id,
                &clm,
                &clm_prime,
                &index.to_le_bytes(),
                &salt,
                &guess.to_le_bytes(),
            ],
        )
    }

    /// Canonical Target transaction for one resolution branch.
    pub fn target_template(
        &self,
        ctx: Context,
        mode: ResolutionMode,
    ) -> Result<Template, CompilationError> {
        let output = self.target_output(mode);
        let height = match mode {
            ResolutionMode::Operator => self.expiry()?,
            ResolutionMode::Default => self.default_height()?,
        };
        let mut plan = ctx.template_plan();
        let sponsor = plan.input("fee_sponsor", Amount::ZERO)?;
        plan.require_older(&plan.contract_input(), RelHeight::from(0).into())?;
        plan.require_older(&sponsor, RelHeight::from(0).into())?;
        plan.require_after(height.into())?;
        plan.output("target", OutputAmount::Exact(self.value), &output)?;
        plan.surplus(Surplus::Fees {
            maximum: self.maximum_sponsor_fee,
        });
        Ok(plan.finish()?)
    }

    /// Canonical Hit transaction: the fixed 50/50 split of the depot into
    /// the insurance branch (output 0) and the claim branch (output 1).
    pub fn hit_template(
        &self,
        ctx: Context,
        mode: ResolutionMode,
    ) -> Result<Template, CompilationError> {
        let insurance = self.hit_insurance_output(mode);
        let claim = self.hit_claim_output(mode);
        let half = self.value / 2;
        let mut plan = ctx.template_plan();
        let sponsor = plan.input("fee_sponsor", Amount::ZERO)?;
        plan.require_older(&plan.contract_input(), RelHeight::from(0).into())?;
        plan.require_older(&sponsor, RelHeight::from(0).into())?;
        plan.output("insurance", OutputAmount::Exact(half), &insurance)?;
        plan.output("claim", OutputAmount::Exact(half), &claim)?;
        plan.surplus(Surplus::Fees {
            maximum: self.maximum_sponsor_fee,
        });
        Ok(plan.finish()?)
    }

    /// Canonical Burn transaction for any half-depot output. The paper locks
    /// burned value for twenty years; this demo provably destroys it.
    pub fn burn_template(&self, ctx: Context) -> Result<Template, CompilationError> {
        let burn = Compiled::from_op_return(&b"depots/v1 burn"[..])?;
        let mut plan = ctx.template_plan();
        let sponsor = plan.input("fee_sponsor", Amount::ZERO)?;
        plan.require_older(&plan.contract_input(), RelHeight::from(0).into())?;
        plan.require_older(&sponsor, RelHeight::from(0).into())?;
        plan.output("burn", OutputAmount::Exact(self.value / 2), &burn)?;
        plan.surplus(Surplus::Fees {
            maximum: self.maximum_sponsor_fee,
        });
        Ok(plan.finish()?)
    }

    /// Canonical claim transaction for one channel: it spends the Hit
    /// transaction's output 1 after `tsd` into the channel's settlement
    /// layer. Its template hash is what the operator blind-signs.
    pub fn claim_template(
        &self,
        ctx: Context,
        user: XOnlyPublicKey,
        mode: ResolutionMode,
    ) -> Result<Template, CompilationError> {
        let settlement = self.claim_output(user, mode);
        let mut plan = ctx.template_plan();
        let sponsor = plan.input("fee_sponsor", Amount::ZERO)?;
        plan.require_older(
            &plan.contract_input(),
            RelHeight::from(self.to_self_delay).into(),
        )?;
        plan.require_older(&sponsor, RelHeight::from(0).into())?;
        plan.output("channel", OutputAmount::Exact(self.value / 2), &settlement)?;
        plan.surplus(Surplus::Fees {
            maximum: self.maximum_sponsor_fee,
        });
        Ok(plan.finish()?)
    }

    /// The pair of claim template hashes `<Clm_Ui, Clm'_Ui>` the operator
    /// blind-signs when selling a channel: one per resolution branch.
    pub fn claim_hashes(
        &self,
        mut ctx: Context,
        user: XOnlyPublicKey,
    ) -> Result<([u8; 32], [u8; 32]), CompilationError> {
        let normal_ctx = ctx.derive_str(Arc::new("clm".into()))?;
        let clm = self.claim_template(normal_ctx, user, ResolutionMode::Operator)?;
        let default_ctx = ctx.derive_str(Arc::new("clm_prime".into()))?;
        let clm_prime = self.claim_template(default_ctx, user, ResolutionMode::Default)?;
        Ok((
            template_hash(&clm.tx, 0, None)
                .map_err(invalid)?
                .to_byte_array(),
            template_hash(&clm_prime.tx, 0, None)
                .map_err(invalid)?
                .to_byte_array(),
        ))
    }

    pub(super) fn template_commitment(
        &self,
        template: &Template,
    ) -> Result<Sha256, CompilationError> {
        template_hash(&template.tx, 0, None).map_err(invalid)
    }
}

/// Witness section proving an operator-selected target.
pub fn target_evidence(target: u64, signature: &[u8; 64]) -> Vec<u8> {
    let mut evidence = Vec::with_capacity(TARGET_SECTION_LENGTH);
    evidence.extend_from_slice(&target.to_le_bytes());
    evidence.extend_from_slice(signature);
    evidence
}

/// One channel's guess tuple as the evaluator consumes it.
#[derive(Clone, Debug)]
pub struct GuessTuple {
    /// The channel's index within the depot.
    pub index: u64,
    /// The salt hiding the guess inside the blind-signed message.
    pub salt: [u8; 32],
    /// The channel's guess in `0..prime`.
    pub guess: u64,
    /// The operator's blind guess signature GuSig for this channel.
    pub signature: [u8; 64],
    /// Template hash of the channel's claim transaction, operator branch.
    pub clm: [u8; 32],
    /// Template hash of the channel's claim transaction, default branch.
    pub clm_prime: [u8; 32],
}

impl GuessTuple {
    /// Serialize the tuple exactly as the evaluator parses it.
    pub fn encode(&self) -> Vec<u8> {
        let mut tuple = Vec::with_capacity(TUPLE_LENGTH);
        tuple.extend_from_slice(&self.index.to_le_bytes());
        tuple.extend_from_slice(&self.salt);
        tuple.extend_from_slice(&self.guess.to_le_bytes());
        tuple.extend_from_slice(&self.signature);
        tuple.extend_from_slice(&self.clm);
        tuple.extend_from_slice(&self.clm_prime);
        tuple
    }
}
