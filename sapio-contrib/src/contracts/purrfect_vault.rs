//! Purrfect vault's shared-address trigger, cancel and complete transaction flow.
//!
//! Every branch requires the same owner signature. A trigger preserves the
//! principal at this address and publishes a separate 546-satoshi destination
//! output. Cancellation preserves the principal and removes that marker.
//! Completion spends output zero of the authenticated trigger, paying the
//! principal to its marker's script after a native twenty-block CSV delay.
//!
//! Actual CAT scripts execute in a pinned WASM Script interpreter before the
//! oracle signs. The owner signature and CSV also remain native Script
//! requirements; the owner has no unconditional key-path escape. All proposals
//! use a separate fee sponsor, preserving the vault's entire principal.

use bitcoin::bip32::Xpub;
use bitcoin::consensus::encode::serialize;
use bitcoin::{Amount, ScriptBuf, Transaction, XOnlyPublicKey};
use sapio::contract::*;
use sapio::template::{OutputAmount, Surplus, Template};
use sapio::util::extended_address::ExtendedAddress;
use sapio_base::policy::ScriptPolicy;
use sapio_base::program::EmulatedProgram;
use sapio_base::serialization_helpers::SArc;
use sapio_base::timelocks::RelHeight;
use sapio_base::Clause;
use std::sync::Arc;

mod script;
pub use script::script;
mod witness;
pub use witness::{challenge, script_witness, synthetic_signature};

/// Destination output value in the original protocol, separately sponsored.
pub const MARKER_SATS: u64 = 546;
/// Native relative block delay required by the completion branch.
pub const WITHDRAWAL_DELAY: u16 = 20;
/// The native OP_CAT reconstruction must fit a single Script stack element.
pub const MAX_TRIGGER_BYTES: usize = 520;

/// Select the CAT Script committed as the evaluator's immutable parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Stage {
    /// Preserve the principal and publish the destination as output one.
    Trigger = 0,
    /// Preserve the principal without a destination marker.
    Cancel = 1,
    /// Authenticate the previous trigger and pay its selected destination.
    Complete = 2,
}

fn invalid(error: impl std::fmt::Display) -> CompilationError {
    CompilationError::Custom(error.to_string().into())
}

/// Encode the transaction-ID preimage supplied as completion evidence.
///
/// Witnesses do not contribute to the txid and must not enter the OP_CAT
/// reconstruction. Native witness-v0 fee sponsors have empty scriptSigs, so
/// their signing cannot change the transaction being authenticated here.
pub fn stripped_trigger(trigger: &Transaction) -> Result<Vec<u8>, CompilationError> {
    if trigger.input.len() != 2
        || trigger
            .input
            .iter()
            .any(|input| !input.script_sig.is_empty())
    {
        return Err(invalid("trigger needs two inputs with empty scriptSigs"));
    }
    if trigger.output.len() != 2 || trigger.output[1].value.to_sat() != MARKER_SATS {
        return Err(invalid(
            "trigger needs principal and 546-satoshi destination outputs",
        ));
    }
    let mut stripped = trigger.clone();
    for input in &mut stripped.input {
        input.witness.clear();
    }
    let evidence = serialize(&stripped);
    if evidence.len() > MAX_TRIGGER_BYTES {
        return Err(invalid(
            "trigger exceeds the native OP_CAT reconstruction limit",
        ));
    }
    Ok(evidence)
}

/// Immutable spending terms for every deposit and trigger at one vault address.
///
/// Principal and proposed destinations are not address parameters. The fee cap
/// is local transaction funding policy, independently of the spending leaves.
#[derive(Clone, Debug)]
pub struct PurrfectVault {
    owner: XOnlyPublicKey,
    oracle_root: Xpub,
    maximum_sponsor_fee: Amount,
}

#[sapio::contract]
impl PurrfectVault {
    /// Bind one owner's native signatures and the emulation oracle's public root.
    pub fn new(
        owner: XOnlyPublicKey,
        oracle_root: Xpub,
        maximum_sponsor_fee: Amount,
    ) -> Result<Self, CompilationError> {
        let vault = Self {
            owner,
            oracle_root,
            maximum_sponsor_fee,
        };
        vault.program(Stage::Trigger)?;
        Ok(vault)
    }

    /// Native authorization key shared by all three branches.
    pub fn owner(&self) -> XOnlyPublicKey {
        self.owner
    }

    /// Public root from which the three predicate-signing keys are derived.
    pub fn oracle_root(&self) -> Xpub {
        self.oracle_root
    }

    /// Local ceiling on sponsor value consumed as fees by each proposal.
    pub fn maximum_sponsor_fee(&self) -> Amount {
        self.maximum_sponsor_fee
    }

    /// The complete, content-addressed transaction predicate for one branch.
    pub fn program(&self, stage: Stage) -> Result<EmulatedProgram, CompilationError> {
        sapio_base::bitcoin_script::emulated_script(&script(stage, self.owner), self.oracle_root)
            .map_err(invalid)
    }

    fn policy(&self, stage: Stage) -> Result<ScriptPolicy, CompilationError> {
        let mut guards = vec![Clause::Key(self.owner).into(), self.program(stage)?.into()];
        if stage == Stage::Complete {
            guards.push(Clause::try_from(RelHeight::from(WITHDRAWAL_DELAY))?.into());
        }
        Ok(ScriptPolicy::And(guards))
    }

    #[policy]
    fn trigger_policy(&self) -> Result<ScriptPolicy, CompilationError> {
        self.policy(Stage::Trigger)
    }

    #[policy]
    fn cancel_policy(&self) -> Result<ScriptPolicy, CompilationError> {
        self.policy(Stage::Cancel)
    }

    #[policy]
    fn complete_policy(&self) -> Result<ScriptPolicy, CompilationError> {
        self.policy(Stage::Complete)
    }

    #[amount]
    fn principal(&self, ctx: Context) -> Result<Amount, CompilationError> {
        Ok(ctx.funds())
    }

    fn principal_output(&self, ctx: &Context) -> Result<Compiled, CompilationError> {
        // Suggested actions have no default requests. Empty effects therefore
        // compile exactly the fixed policies, without recursively proposing
        // another trigger or cancellation at this same address.
        let identity = self.compile(Context::new(
            ctx.network,
            ctx.funds(),
            ctx.lowering_plan().clone(),
            ctx.path().as_ref().clone(),
            Arc::new(Default::default()),
            None,
        ))?;
        Ok(script_output((&identity.address).into(), ctx))
    }

    /// Propose a destination, separately funding its marker and all miner fees.
    ///
    /// Use a native witness-v0 fee sponsor so every input has an empty scriptSig.
    /// Its value must cover `MARKER_SATS` plus the desired transaction fee.
    #[action(suggested, guarded_by(Self::trigger_policy))]
    pub fn trigger(
        &self,
        ctx: Context,
        destination: ScriptBuf,
    ) -> Result<Template, CompilationError> {
        let principal = ctx.funds();
        let own_output = self.principal_output(&ctx)?;
        let target = destination_output(destination, &ctx)?;
        let mut plan = ctx.template_plan();
        plan.input("fee_sponsor", Amount::from_sat(MARKER_SATS))?;
        plan.output("principal", OutputAmount::Exact(principal), &own_output)?;
        plan.output(
            "destination_marker",
            OutputAmount::Exact(Amount::from_sat(MARKER_SATS)),
            &target,
        )?;
        plan.surplus(Surplus::Fees {
            maximum: self.maximum_sponsor_fee,
        });
        let template = plan.finish()?;
        // Reject destinations whose trigger could never be completed by the
        // native reconstruction. Binding preserves this transaction's size.
        stripped_trigger(&template.tx)?;
        Ok(template)
    }

    /// Remove a pending destination while preserving the principal and address.
    #[action(suggested, guarded_by(Self::cancel_policy))]
    pub fn cancel(&self, ctx: Context) -> Result<Template, CompilationError> {
        let principal = ctx.funds();
        let own_output = self.principal_output(&ctx)?;
        let mut plan = ctx.template_plan();
        plan.input("fee_sponsor", Amount::ZERO)?;
        plan.output("principal", OutputAmount::Exact(principal), &own_output)?;
        plan.surplus(Surplus::Fees {
            maximum: self.maximum_sponsor_fee,
        });
        Ok(plan.finish()?)
    }

    /// Pay the authenticated trigger's destination after its twenty-block delay.
    ///
    /// The evaluator checks that input zero spends this transaction's output
    /// zero. Bind the proposal to that outpoint and construct its Script stack
    /// when requesting the oracle signature; no destination is trusted locally.
    #[action(suggested, guarded_by(Self::complete_policy))]
    pub fn complete(
        &self,
        ctx: Context,
        previous_trigger: Transaction,
    ) -> Result<Template, CompilationError> {
        stripped_trigger(&previous_trigger)?;
        let principal = ctx.funds();
        let own_output = self.principal_output(&ctx)?;
        let own_script: ScriptBuf = (&own_output.address).into();
        if previous_trigger.output[0].value != principal
            || previous_trigger.output[0].script_pubkey != own_script
        {
            return Err(invalid(
                "trigger principal does not match this funded vault",
            ));
        }
        let target = destination_output(previous_trigger.output[1].script_pubkey.clone(), &ctx)?;
        let mut plan = ctx.template_plan();
        plan.input("fee_sponsor", Amount::ZERO)?;
        plan.require_older(
            &plan.contract_input(),
            RelHeight::from(WITHDRAWAL_DELAY).into(),
        )?;
        plan.output("withdrawal", OutputAmount::Exact(principal), &target)?;
        plan.surplus(Surplus::Fees {
            maximum: self.maximum_sponsor_fee,
        });
        Ok(plan.finish()?)
    }
}

// Destinations are Script bytes, as in the original covenant, rather than a
// restricted address class. They carry no asserted descriptor or spend policy.
fn script_output(script: ScriptBuf, ctx: &Context) -> Compiled {
    Compiled {
        nums_internal_key: None,
        committed_policy_guards: Default::default(),
        alternative_policies: Vec::new(),
        program_policies: Vec::new(),
        covenant_requirements: Default::default(),
        ctv_to_tx: Default::default(),
        suggested_txs: Default::default(),
        continue_apis: Default::default(),
        root_path: SArc(ctx.path().clone()),
        address: ExtendedAddress::Unknown(script),
        descriptor: None,
        required_input_amount: Amount::ZERO,
        metadata: Default::default(),
    }
}

fn destination_output(script: ScriptBuf, ctx: &Context) -> Result<Compiled, CompilationError> {
    if !(1..=127).contains(&script.len()) {
        return Err(invalid("CAT destinations must contain 1..127 Script bytes"));
    }
    Ok(script_output(script, ctx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{address, context, key};
    use bitcoin::bip32::Xpriv;
    use bitcoin::Network;
    use sapio_base::effects::EffectPath;
    use sapio_base::secp::secp;

    fn vault() -> PurrfectVault {
        let root = Xpriv::new_master(Network::Testnet, &[71; 32]).unwrap();
        PurrfectVault::new(
            key(72),
            Xpub::from_priv(secp(), &root),
            Amount::from_sat(2_000),
        )
        .unwrap()
    }

    #[test]
    fn proposals_preserve_the_shared_address_and_principal() {
        let vault = vault();
        let bare = vault.compile(context(100_000)).unwrap();
        assert!(bare.suggested_txs.is_empty());
        assert!(bare.ctv_to_tx.is_empty());
        assert_eq!(bare.program_policies.len(), 3);
        assert_eq!(
            bare.address,
            vault.compile(context(200_000)).unwrap().address
        );
        assert_eq!(
            bare.address,
            PurrfectVault::new(vault.owner(), vault.oracle_root(), Amount::ZERO)
                .unwrap()
                .compile(context(100_000))
                .unwrap()
                .address
        );
        assert!(bare
            .program_requirements()
            .unwrap()
            .iter()
            .all(|requirement| matches!(
                requirement.path,
                sapio_base::program::ProgramSpendPath::ScriptPath(_)
            )));
        let destination = address(73).script_pubkey();
        let trigger = vault
            .trigger(context(100_000), destination.clone())
            .unwrap();
        let cancel = vault.cancel(context(100_000)).unwrap();
        let complete = vault
            .complete(context(100_000), trigger.tx.clone())
            .unwrap();
        let script: ScriptBuf = (&bare.address).into();
        assert_eq!(trigger.tx.output[0].script_pubkey, script);
        assert_eq!(cancel.tx.output[0].script_pubkey, script);
        assert_eq!(trigger.tx.output[1].script_pubkey, destination);
        assert_eq!(trigger.tx.output[1].value.to_sat(), MARKER_SATS);
        assert_eq!(complete.tx.output[0].script_pubkey, destination);
        for template in [&trigger, &cancel, &complete] {
            assert_eq!(template.required_input_amount.to_sat(), 100_000);
            assert_eq!(template.tx.output[0].value.to_sat(), 100_000);
            assert_eq!(template.tx.input.len(), 2);
            template.validate_funding_constraints().unwrap();
        }
        assert_eq!(complete.tx.input[0].sequence.to_consensus_u32(), 20);
        let effects = PurrfectVault::trigger_action()
            .request(&EffectPath::try_from("example").unwrap(), &destination)
            .unwrap();
        let proposed = vault
            .compile(Context::new(
                Network::Regtest,
                Amount::from_sat(100_000),
                sapio_base::LoweringPlan::Native,
                EffectPath::try_from("example").unwrap(),
                Arc::new(effects),
                None,
            ))
            .unwrap();
        assert_eq!(bare.address, proposed.address);
        assert_eq!(proposed.suggested_txs.len(), 1);
        proposed.validate().unwrap();
    }

    #[test]
    fn completion_authenticates_the_full_marker_and_strips_witnesses() {
        let vault = vault();
        // An arbitrary Script destination remains supported, without inventing
        // a P2TR-only restriction on the protocol's serialized marker.
        let destination = ScriptBuf::from_bytes(vec![0x51]);
        let trigger = vault
            .trigger(context(100_000), destination.clone())
            .unwrap();
        let mut witnessed = trigger.tx.clone();
        witnessed.input[0].witness.push([1; 64]);
        assert_eq!(
            stripped_trigger(&witnessed).unwrap(),
            serialize(&trigger.tx)
        );
        let complete = vault.complete(context(100_000), witnessed).unwrap();
        assert_eq!(complete.tx.output[0].script_pubkey, destination);
        let mut wrong_marker = trigger.tx.clone();
        wrong_marker.output[1].value = Amount::from_sat(MARKER_SATS - 1);
        assert!(vault.complete(context(100_000), wrong_marker).is_err());
        assert!(vault.complete(context(99_999), trigger.tx.clone()).is_err());
        let mut changed_principal = trigger.tx;
        changed_principal.output[0].script_pubkey = address(74).script_pubkey();
        assert!(vault.complete(context(100_000), changed_principal).is_err());
        assert!(vault
            .trigger(context(100_000), ScriptBuf::from_bytes(vec![0x51; 520]))
            .is_err());
    }
}
