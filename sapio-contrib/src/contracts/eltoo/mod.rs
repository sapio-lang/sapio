//! An eltoo-style channel using TemplateHash, internal-key authorization and CSFS.
//!
//! The contract contains public terms and spending rules. Compilation drivers,
//! signing, fee sponsorship and chain recovery live in the integration example.

mod terms;
pub use terms::{State, Terms, LOCK_TIME_BASE, MAX_STATE, RECOVERY_TAG};

use bitcoin::taproot::{LeafVersion, TapLeafHash};
use bitcoin::{Amount, ScriptBuf, XOnlyPublicKey};
use sapio::contract::abi::object::ObjectMetadata;
use sapio::contract::actions::ConditionalCompileType;
use sapio::contract::compiler::compile_policy_leaf;
use sapio::contract::*;
use sapio::*;
use sapio_base::policy::{PolicyCompiler, ScriptPolicy};
use sapio_base::program::EmulatedProgram;
use sapio_base::timelocks::{AbsTime, RelHeight};
use sapio_base::Clause;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A proposed transaction, independent of the output's fixed spending policy.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub enum Candidate {
    /// Advance the state and publish its settlement commitment.
    Update(State),
    /// Pay the current allocation after the contest period.
    Settle,
}

/// Funding or an agreed state under one immutable set of channel terms.
#[derive(Clone, Debug)]
pub struct Channel {
    terms: Terms,
    state: Option<State>,
}

impl Channel {
    /// Borrow the fixed channel terms.
    pub fn terms(&self) -> &Terms {
        &self.terms
    }
    /// Funding has no state and cannot settle unilaterally.
    pub fn current_state(&self) -> Option<State> {
        self.state
    }

    #[guard(cached)]
    fn cooperative(self) {
        Clause::Key(self.terms.joint_key())
    }

    #[guard(policy, cached)]
    fn update_policy(self) -> Result<ScriptPolicy, CompilationError> {
        self.terms
            .update_policy(self.state.map(|state| state.number))
    }

    #[guard(policy)]
    fn settlement_policy(self, ctx: Context) -> Result<ScriptPolicy, CompilationError> {
        Ok(ScriptPolicy::And(vec![
            Clause::try_from(RelHeight::from(self.terms.delay()))?.into(),
            self.settlement_program(ctx)?.compile_policy()?,
        ]))
    }

    #[compile_if]
    fn can_update(self, _ctx: Context) {
        if self
            .state
            .is_some_and(|state| state.number == self.terms.max_state())
        {
            ConditionalCompileType::Never
        } else {
            ConditionalCompileType::NoConstraint
        }
    }

    #[compile_if]
    fn can_settle(self, _ctx: Context) {
        if self.state.is_some() {
            ConditionalCompileType::NoConstraint
        } else {
            ConditionalCompileType::Never
        }
    }

    #[continuation(
        guarded_by = "[Self::update_policy]",
        compile_if = "[Self::can_update]",
        web_api
    )]
    fn update(self, mut ctx: Context, candidate: Option<Candidate>) {
        let Some(Candidate::Update(state)) = candidate else {
            return empty();
        };
        if self.state.is_some_and(|old| state.number <= old.number) {
            return Err(terms::invalid("updates must advance the state number"));
        }
        let successor = self.terms.state(state)?;
        // Publish the sibling needed to recover the next update proof without
        // retaining this allocation. Commit the compiler's actual leaf encoding.
        let leaf = successor.settlement_script(
            ctx.derive_str(std::sync::Arc::new("settlement_commitment".into()))?,
        )?;
        let mut payload = RECOVERY_TAG.to_vec();
        payload.extend_from_slice(TapLeafHash::from_script(&leaf, LeafVersion::TapScript).as_ref());
        let recovery = Compiled::from_op_return(&payload)?;
        ctx.template()
            .add_sequence()
            .set_sequence(0, RelHeight::from(0).into())?
            .set_sequence(1, RelHeight::from(0).into())?
            .set_lock_time(AbsTime::try_from(self.terms.lock_time(state.number)?)?.into())?
            .add_output(Amount::from_sat(self.terms.capacity()), &successor, None)?
            .add_output(Amount::ZERO, &recovery, None)?
            .into()
    }

    #[continuation(
        guarded_by = "[Self::settlement_policy]",
        compile_if = "[Self::can_settle]",
        web_api
    )]
    fn settle(self, ctx: Context, candidate: Option<Candidate>) {
        let Some(Candidate::Settle) = candidate else {
            return empty();
        };
        let state = self
            .state
            .ok_or_else(|| terms::invalid("funding cannot settle"))?;
        Ok(Box::new(std::iter::once(
            self.terms.settlement_template(state, ctx),
        )))
    }

    /// The exact settlement predicate, built using the caller's compilation context.
    pub fn settlement_program(&self, ctx: Context) -> Result<EmulatedProgram, CompilationError> {
        let state = self
            .state
            .ok_or_else(|| terms::invalid("funding cannot settle"))?;
        self.terms.settlement_program(state, ctx)
    }

    /// Canonical settlement leaf used by the update's recovery publication.
    pub fn settlement_script(&self, ctx: Context) -> Result<ScriptBuf, CompilationError> {
        compile_policy_leaf(&self.guard_settlement_policy(ctx))
    }
}

impl Contract for Channel {
    declare! {finish, Self::cooperative}
    declare! {actions, Self::update, Self::settle}

    fn ensure_amount(&self, _ctx: Context) -> Result<Amount, CompilationError> {
        Ok(Amount::from_sat(self.terms.capacity()))
    }

    fn pinned_internal_key(
        &self,
        _ctx: &Context,
    ) -> Result<Option<XOnlyPublicKey>, CompilationError> {
        Ok(Some(self.terms.joint_key()))
    }

    fn metadata(&self, _ctx: Context) -> Result<ObjectMetadata, CompilationError> {
        let mut metadata = ObjectMetadata::default();
        metadata.extra.insert(
            "eltoo".into(),
            serde_json::json!({
                "terms": {
                    "joint_key": self.terms.joint_key(),
                    "capacity": self.terms.capacity(),
                    "delay": self.terms.delay(),
                    "max_state": self.terms.max_state(),
                    "alice": self.terms.alice(),
                    "bob": self.terms.bob(),
                },
                "state": self.state,
            }),
        );
        Ok(metadata)
    }
}
