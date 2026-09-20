// Copyright Judica, Inc 2026
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! # Depots
//!
//! The depot protocol from jlspc's "Depots: Theft-Proof, Self-Custodial
//! Bitcoin For Billions Of Users"
//! (<https://delvingbitcoin.org/t/depots-theft-proof-self-custodial-bitcoin-for-billions-of-users/2892>),
//! Section 3.1, Figures 1 and 2.
//!
//! An operator funds a single output backing probabilistic Lightning
//! channels. A channel is a blind operator signature over `<Clm_Ui,
//! Clm'_Ui, i, Salt_i, G_i>`: the channel's two claim-transaction hashes,
//! its index, a salt, and a secret guess in `0..P`. The depot's address
//! commits to **no user set** — channels, guesses and claim bindings live
//! entirely in signature evidence, so one depot serves any number of users
//! with a constant on-chain footprint.
//!
//! After expiry `E` the operator reveals a target `t` by signature and the
//! depot resolves along Figure 1: `F -> Targ -> Hit`, where any channel
//! whose guess hits `t` can force the fixed 50/50 split, then bind its
//! exact blind-signed claim transaction (a CTV whose hash arrives in the
//! witness, authenticated by CSFS) after `tsd`. Two distinct guess
//! signatures for the same target burn the funds — the griefing penalty
//! that makes theft irrational for both sides, including the operator's
//! `Arb` equivocation, which necessarily reveals a second signature. If the
//! operator withholds the target for `tsd` past expiry, Figure 2's default
//! branch (`Targ'`, target fixed at zero) lets any user resolve the depot
//! without the operator.
//!
//! ## Emulation
//!
//! The protocol requires CTV and CSFS (and benefits from PairCommit). This
//! implementation lowers those predicates to `EmulatedProgram` leaves
//! running the [depots evaluator](../../../../evaluators/depots/src/lib.rs)
//! under Sapio's program oracle: each leaf is an oracle-derived key whose
//! signature is produced only after the metered WASM predicate validates
//! the target signature, guess signature, hit equality and template
//! commitment against the signed transaction view. Reveals travel as
//! off-chain evidence to the oracle rather than on-chain witness data; an
//! operator who withholds them is covered by the default branch exactly as
//! in the paper.
//!
//! Modeling choices: PairCommit nesting becomes one tagged hash; the salt
//! is 32 caller-chosen bytes (the paper uses a joint signature for
//! PTLC-atomic revocation); burns are OP_RETURN outputs instead of
//! twenty-year locks; the channel interior behind a claim is a final
//! `tsd`-delayed 2-of-2 settlement rather than full Lightning commitment
//! machinery; and the single-target simplified protocol is implemented —
//! the paper's multi-target vectors and randomization factors (Section
//! 3.2) refine sale scale and revocation privacy off-chain.

mod terms;
pub use terms::{
    target_evidence, DepotTerms, GuessTuple, ResolutionMode, Stage, DEPOTS_WASM, PARAMETERS_LENGTH,
    TARGET_SECTION_LENGTH, TUPLE_LENGTH,
};

use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::Hash as _;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{Amount, XOnlyPublicKey};
use sapio::contract::abi::object::ObjectMetadata;
use sapio::contract::*;
use sapio::template::{OutputAmount, Surplus, Template};
use sapio_base::fragments::template_hash_eq;
use sapio_base::policy::ScriptPolicy;
use sapio_base::timelocks::RelHeight;
use sapio_base::Clause;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;

/// The depot Funding output `F`: resolvable by the operator's target reveal
/// after expiry, or by anyone along the default branch `tsd` later.
#[derive(Clone, Debug)]
pub struct Depot {
    pub(crate) terms: DepotTerms,
}

#[sapio::contract]
impl Depot {
    /// Borrow the fixed depot terms.
    pub fn terms(&self) -> &DepotTerms {
        &self.terms
    }

    #[policy]
    fn reveal_policy(&self, ctx: Context) -> Result<ScriptPolicy, CompilationError> {
        let template = self.terms.target_template(ctx, ResolutionMode::Operator)?;
        let expected = self.terms.template_commitment(&template)?;
        Ok(ScriptPolicy::And(vec![
            Clause::try_from(self.terms.expiry()?)?.into(),
            self.terms
                .program(Stage::TargetReveal, ResolutionMode::Operator, expected)?
                .into(),
        ]))
    }

    #[policy]
    fn default_policy(&self, ctx: Context) -> Result<ScriptPolicy, CompilationError> {
        let template = self.terms.target_template(ctx, ResolutionMode::Default)?;
        let expected = self.terms.template_commitment(&template)?;
        Ok(ScriptPolicy::And(vec![
            Clause::try_from(self.terms.default_height()?)?.into(),
            template_hash_eq(expected, *self.terms.oracle_root())
                .map_err(terms::invalid)?
                .into(),
        ]))
    }

    /// `Targ`: the operator reveals a target `t` by supplying TaSig evidence.
    #[action(suggested, guarded_by(Self::reveal_policy))]
    pub fn reveal_target(&self, ctx: Context) -> Result<Template, CompilationError> {
        self.terms.target_template(ctx, ResolutionMode::Operator)
    }

    /// `Targ'`: after `tsd` grace, anyone resolves with the target fixed at 0.
    #[action(suggested, guarded_by(Self::default_policy))]
    pub fn default_target(&self, ctx: Context) -> Result<Template, CompilationError> {
        self.terms.target_template(ctx, ResolutionMode::Default)
    }

    #[amount]
    fn ensure_amount(&self, _ctx: Context) -> Result<Amount, CompilationError> {
        Ok(self.terms.value())
    }

    #[metadata]
    fn metadata(&self, _ctx: Context) -> Result<ObjectMetadata, CompilationError> {
        let mut metadata = ObjectMetadata::default();
        metadata.extra.insert(
            "depot".into(),
            serde_json::json!({
                "depot_id": bitcoin::hashes::hex::DisplayHex::to_lower_hex_string(
                    &self.terms.depot_id()[..]
                ),
                "operator": self.terms.operator(),
                "target_key": self.terms.target_key(),
                "guess_key": self.terms.guess_key(),
                "value_sats": self.terms.value().to_sat(),
                "prime": self.terms.prime(),
                "expiry": self.terms.expiry_height(),
                "to_self_delay": self.terms.to_self_delay(),
            }),
        );
        Ok(metadata)
    }
}

/// The Target transaction's single output: swept by the operator after
/// `tsd` when no channel hits, or split by any hit owner.
#[derive(Clone, Debug)]
pub struct TargetOutput {
    pub(crate) terms: DepotTerms,
    pub(crate) mode: ResolutionMode,
}

#[sapio::contract]
impl TargetOutput {
    /// This branch's resolution mode.
    pub fn mode(&self) -> ResolutionMode {
        self.mode
    }

    /// No hit: the operator reclaims the depot after the monitoring window.
    #[spend]
    fn operator_sweep(&self) -> Result<ScriptPolicy, CompilationError> {
        Ok(ScriptPolicy::Miniscript(Clause::And(vec![
            Clause::Key(self.terms.operator()).into(),
            Clause::try_from(RelHeight::from(self.terms.to_self_delay()))?.into(),
        ])))
    }

    #[policy]
    fn hit_policy(&self, ctx: Context) -> Result<ScriptPolicy, CompilationError> {
        let template = self.terms.hit_template(ctx, self.mode)?;
        let expected = self.terms.template_commitment(&template)?;
        Ok(self.terms.program(Stage::Hit, self.mode, expected)?.into())
    }

    /// `Hit`: a hit owner's evidence forces the fixed 50/50 split.
    #[action(suggested, guarded_by(Self::hit_policy))]
    pub fn hit(&self, ctx: Context) -> Result<Template, CompilationError> {
        self.terms.hit_template(ctx, self.mode)
    }

    #[amount]
    fn ensure_amount(&self, _ctx: Context) -> Result<Amount, CompilationError> {
        Ok(self.terms.value())
    }
}

/// Hit output 0: the insurance half. It burns on proof of a double hit and
/// otherwise returns to the operator after an extended window.
#[derive(Clone, Debug)]
pub struct HitInsuranceOutput {
    pub(crate) terms: DepotTerms,
    pub(crate) mode: ResolutionMode,
}

#[sapio::contract]
impl HitInsuranceOutput {
    /// Single hit: the operator reclaims the insurance half after `3*tsd`.
    #[spend]
    fn operator_reclaim(&self) -> Result<ScriptPolicy, CompilationError> {
        Ok(ScriptPolicy::Miniscript(Clause::And(vec![
            Clause::Key(self.terms.operator()).into(),
            Clause::try_from(RelHeight::from(3 * self.terms.to_self_delay()))?.into(),
        ])))
    }

    #[policy]
    fn burn_policy(&self, ctx: Context) -> Result<ScriptPolicy, CompilationError> {
        let template = self.terms.burn_template(ctx)?;
        let expected = self.terms.template_commitment(&template)?;
        Ok(self.terms.program(Stage::Burn, self.mode, expected)?.into())
    }

    /// Two distinct guess signatures for the target destroy this half.
    #[action(suggested, guarded_by(Self::burn_policy))]
    pub fn burn(&self, ctx: Context) -> Result<Template, CompilationError> {
        self.terms.burn_template(ctx)
    }

    #[amount]
    fn ensure_amount(&self, _ctx: Context) -> Result<Amount, CompilationError> {
        Ok(self.terms.value() / 2)
    }
}

/// A request to bind one channel's claim transaction to the claim branch.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClaimRequest {
    /// The claiming channel's user key.
    pub user: XOnlyPublicKey,
}

/// Hit output 1: the claim half. After `tsd`, the exact claim transaction
/// bound inside a valid guess signature can spend it; a double hit burns it.
/// The operator's `Arb` escape uses this same leaf and necessarily reveals
/// a second guess signature, arming the insurance burn.
#[derive(Clone, Debug)]
pub struct HitClaimOutput {
    pub(crate) terms: DepotTerms,
    pub(crate) mode: ResolutionMode,
}

#[sapio::contract]
impl HitClaimOutput {
    #[policy]
    fn burn_policy(&self, ctx: Context) -> Result<ScriptPolicy, CompilationError> {
        let template = self.terms.burn_template(ctx)?;
        let expected = self.terms.template_commitment(&template)?;
        Ok(self.terms.program(Stage::Burn, self.mode, expected)?.into())
    }

    /// Two distinct guess signatures for the target destroy this half.
    #[action(suggested, guarded_by(Self::burn_policy))]
    pub fn burn(&self, ctx: Context) -> Result<Template, CompilationError> {
        self.terms.burn_template(ctx)
    }

    #[policy]
    fn claim_policy(&self, _ctx: Context) -> Result<ScriptPolicy, CompilationError> {
        Ok(ScriptPolicy::And(vec![
            Clause::try_from(RelHeight::from(self.terms.to_self_delay()))?.into(),
            self.terms
                .program(Stage::Claim, self.mode, Sha256::from_byte_array([0u8; 32]))?
                .into(),
        ]))
    }

    /// `Clm_Ui`: the claim transaction the operator blind-signed for the
    /// hitting channel. The covenant's expected hash arrives in evidence,
    /// authenticated by the guess signature.
    #[action(suggested, guarded_by(Self::claim_policy))]
    pub fn claim(&self, ctx: Context, request: ClaimRequest) -> Result<Template, CompilationError> {
        self.terms.claim_template(ctx, request.user, self.mode)
    }

    #[amount]
    fn ensure_amount(&self, _ctx: Context) -> Result<Amount, CompilationError> {
        Ok(self.terms.value() / 2)
    }
}

/// A settled channel balance: the user's share of the claimed half.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ChannelBalance {
    /// The user's allocation in satoshis; the operator receives the rest.
    pub user_sats: u64,
}

/// The claim transaction's output: the channel's settlement layer. The
/// parties place the final commitment after `tsd`; a discovered double hit
/// can still burn it.
#[derive(Clone, Debug)]
pub struct ClaimOutput {
    pub(crate) terms: DepotTerms,
    pub(crate) user: XOnlyPublicKey,
    pub(crate) mode: ResolutionMode,
}

#[sapio::contract]
impl ClaimOutput {
    /// The claiming channel's user key.
    pub fn user(&self) -> XOnlyPublicKey {
        self.user
    }

    #[policy]
    fn burn_policy(&self, ctx: Context) -> Result<ScriptPolicy, CompilationError> {
        let template = self.terms.burn_template(ctx)?;
        let expected = self.terms.template_commitment(&template)?;
        Ok(self.terms.program(Stage::Burn, self.mode, expected)?.into())
    }

    /// Two distinct guess signatures for the target destroy the channel.
    #[action(suggested, guarded_by(Self::burn_policy))]
    pub fn burn(&self, ctx: Context) -> Result<Template, CompilationError> {
        self.terms.burn_template(ctx)
    }

    #[policy]
    fn settle_policy(&self, _ctx: Context) -> Result<ScriptPolicy, CompilationError> {
        Ok(ScriptPolicy::And(vec![
            Clause::try_from(RelHeight::from(self.terms.to_self_delay()))?.into(),
            ScriptPolicy::Miniscript(Clause::And(vec![
                Clause::Key(self.user).into(),
                Clause::Key(self.terms.operator()).into(),
            ])),
        ]))
    }

    /// `Com`: both parties place the final channel state after `tsd`.
    #[action(suggested, guarded_by(Self::settle_policy))]
    pub fn commit(
        &self,
        ctx: Context,
        state: ChannelBalance,
    ) -> Result<Template, CompilationError> {
        let half = self.terms.value() / 2;
        if state.user_sats > half.to_sat() {
            return Err(terms::invalid("channel state exceeds the claimed half"));
        }
        let secp = Secp256k1::verification_only();
        let network = ctx.network;
        let payout = |key: XOnlyPublicKey| {
            Compiled::from_address(
                bitcoin::Address::p2tr(&secp, key, None, network),
                Amount::ZERO,
            )
        };
        let user_payout = payout(self.user);
        let operator_payout = payout(self.terms.operator());
        let mut plan = ctx.template_plan();
        let sponsor = plan.input("fee_sponsor", Amount::ZERO)?;
        plan.require_older(
            &plan.contract_input(),
            RelHeight::from(self.terms.to_self_delay()).into(),
        )?;
        plan.require_older(&sponsor, RelHeight::from(0).into())?;
        for (name, amount, recipient) in [
            ("user", state.user_sats, &user_payout),
            (
                "operator",
                half.to_sat() - state.user_sats,
                &operator_payout,
            ),
        ] {
            if amount != 0 {
                plan.output(
                    name,
                    OutputAmount::Exact(Amount::from_sat(amount)),
                    recipient,
                )?;
            }
        }
        plan.surplus(Surplus::Fees {
            maximum: self.terms.maximum_sponsor_fee(),
        });
        Ok(plan.finish()?)
    }

    #[amount]
    fn ensure_amount(&self, _ctx: Context) -> Result<Amount, CompilationError> {
        Ok(self.terms.value() / 2)
    }
}
