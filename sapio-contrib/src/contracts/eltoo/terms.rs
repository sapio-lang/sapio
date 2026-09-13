//! Public channel parameters and canonical transition templates.

use super::Channel;
use bitcoin::bip32::Xpub;
use bitcoin::{Address, Amount, ScriptBuf};
use sapio::contract::compiler::compile_policy_leaf;
use sapio::contract::{CompilationError, Compiled, Context};
use sapio::template::{OutputAmount, Surplus, Template};
use sapio_base::fragments::{template_hash, template_hash_eq, template_signed_by, TemplateKey};
use sapio_base::policy::{PolicyCompiler, ScriptPolicy};
use sapio_base::program::EmulatedProgram;
use sapio_base::timelocks::{AbsTime, RelHeight};
use sapio_base::Clause;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// State locks use historical timestamps, independently of the host's clock.
pub const LOCK_TIME_BASE: u32 = 500_000_000;
/// Upper bound keeps every state timestamp within that historical interval.
pub const MAX_STATE: u32 = 1_000_000;
/// Recovery publication tag, followed by the settlement leaf hash.
pub const RECOVERY_TAG: &[u8; 8] = b"eltoo/v1";

pub(super) fn invalid(error: impl std::fmt::Display) -> CompilationError {
    CompilationError::Custom(error.to_string().into())
}

/// One agreed allocation. Channel construction checks its number and balance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct State {
    /// Strictly increasing state number within the channel's interval.
    pub number: u32,
    /// Alice's allocation; Bob receives the rest of the capacity.
    pub alice_sats: u64,
}

/// Immutable public terms. Each channel must use a fresh joint authorization key.
#[derive(Clone, Debug, Serialize)]
pub struct Terms {
    joint_key: bitcoin::XOnlyPublicKey,
    capacity: u64,
    delay: u16,
    max_state: u32,
    #[serde(
        rename = "maximum_sponsor_fee_sats",
        with = "bitcoin::amount::serde::as_sat"
    )]
    maximum_sponsor_fee: Amount,
    alice: Address,
    bob: Address,
    update_program: EmulatedProgram,
}

impl Terms {
    /// Set authorization, destinations, capacity and the bounded contest rules.
    pub fn new(
        joint_key: bitcoin::XOnlyPublicKey,
        oracle_root: Xpub,
        capacity: u64,
        delay: u16,
        max_state: u32,
        maximum_sponsor_fee: Amount,
        alice: Address,
        bob: Address,
    ) -> Result<Self, CompilationError> {
        if capacity == 0 || capacity > 21_000_000 * 100_000_000 {
            return Err(invalid(
                "channel capacity is outside the Bitcoin money range",
            ));
        }
        if delay == 0 {
            return Err(invalid("settlement delay must be positive"));
        }
        if maximum_sponsor_fee == Amount::ZERO {
            return Err(invalid("fee sponsor ceiling must be positive"));
        }
        if !(1..=MAX_STATE).contains(&max_state) {
            return Err(invalid("state interval must end between 1 and MAX_STATE"));
        }
        let update_program =
            template_signed_by(TemplateKey::InternalKey, oracle_root).map_err(invalid)?;
        Ok(Self {
            joint_key,
            capacity,
            delay,
            max_state,
            maximum_sponsor_fee,
            alice,
            bob,
            update_program,
        })
    }

    /// Joint key pinned as the explicitly cooperative Taproot internal key.
    pub fn joint_key(&self) -> bitcoin::XOnlyPublicKey {
        self.joint_key
    }
    /// Value preserved by every update and settlement, in satoshis.
    pub fn capacity(&self) -> u64 {
        self.capacity
    }
    /// Settlement contest period in blocks.
    pub fn delay(&self) -> u16 {
        self.delay
    }
    /// Last state, with settlement as its only unilateral path.
    pub fn max_state(&self) -> u32 {
        self.max_state
    }
    /// Local ceiling for the sponsor value consumed as a fee; not a Script rule.
    pub fn maximum_sponsor_fee(&self) -> Amount {
        self.maximum_sponsor_fee
    }
    /// Alice's checked destination.
    pub fn alice(&self) -> &Address {
        &self.alice
    }
    /// Bob's checked destination.
    pub fn bob(&self) -> &Address {
        &self.bob
    }
    /// Public evaluator source retained for later authorization requests.
    pub fn update_program(&self) -> &EmulatedProgram {
        &self.update_program
    }

    /// Convert an admitted state number to its fixed historical timestamp.
    pub fn lock_time(&self, number: u32) -> Result<u32, CompilationError> {
        if !(1..=self.max_state).contains(&number) {
            return Err(invalid("state number lies outside this channel's interval"));
        }
        Ok(LOCK_TIME_BASE + number)
    }

    /// Check the agreed state against these terms.
    pub fn validate(&self, state: State) -> Result<(), CompilationError> {
        self.lock_time(state.number)?;
        if state.alice_sats > self.capacity {
            return Err(invalid("state allocation exceeds channel capacity"));
        }
        Ok(())
    }

    /// Funding permits updates and cooperative closure, without a settlement leaf.
    pub fn funding(&self) -> Channel {
        Channel {
            terms: self.clone(),
            state: None,
        }
    }

    /// Construct public contract source without compiling it or contacting a signer.
    pub fn state(&self, state: State) -> Result<Channel, CompilationError> {
        self.validate(state)?;
        Ok(Channel {
            terms: self.clone(),
            state: Some(state),
        })
    }

    pub(super) fn update_policy(
        &self,
        number: Option<u32>,
    ) -> Result<ScriptPolicy, CompilationError> {
        let program = self.update_program.compile_policy()?;
        Ok(match number {
            None => program,
            Some(number) => ScriptPolicy::And(vec![
                Clause::try_from(AbsTime::try_from(self.lock_time(number)? + 1)?)?.into(),
                program,
            ]),
        })
    }

    /// Recover the update leaf from its counter alone; no old allocation is needed.
    pub fn update_script(&self, number: u32) -> Result<ScriptBuf, CompilationError> {
        self.lock_time(number)?;
        if number == self.max_state {
            return Err(invalid("maximum state cannot update"));
        }
        compile_policy_leaf(&self.update_policy(Some(number))?)
    }

    /// Canonical payout template. Its second input is reserved for a fee sponsor.
    /// The runner supplies that coin; the contract never reduces channel funds for fees.
    pub fn settlement_template(
        &self,
        state: State,
        ctx: Context,
    ) -> Result<Template, CompilationError> {
        self.validate(state)?;
        let alice = Compiled::from_address(self.alice.clone(), Amount::ZERO);
        let bob = Compiled::from_address(self.bob.clone(), Amount::ZERO);
        let mut plan = ctx.template_plan();
        let sponsor = plan.input("fee_sponsor", Amount::ZERO)?;
        plan.require_older(&plan.contract_input(), RelHeight::from(self.delay).into())?;
        plan.require_older(&sponsor, RelHeight::from(0).into())?;
        plan.require_after(AbsTime::try_from(self.lock_time(state.number)?)?.into())?;
        for (name, amount, recipient) in [
            ("alice", state.alice_sats, &alice),
            ("bob", self.capacity - state.alice_sats, &bob),
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
            maximum: self.maximum_sponsor_fee,
        });
        Ok(plan.finish()?)
    }

    pub(super) fn settlement_program(
        &self,
        state: State,
        ctx: Context,
    ) -> Result<EmulatedProgram, CompilationError> {
        let transaction = self.settlement_template(state, ctx)?.tx;
        let hash = template_hash(&transaction, 0, None).map_err(invalid)?;
        template_hash_eq(hash, *self.update_program.root()).map_err(invalid)
    }
}
