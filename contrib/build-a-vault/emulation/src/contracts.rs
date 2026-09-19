//! Vault and pending-withdrawal spending rules, without runner or signer code.

use crate::{invalid, OpVault, WithdrawalProposal};
use bitcoin::taproot::{ControlBlock, LeafVersion, TapNodeHash};
use bitcoin::{Amount, ScriptBuf};
use build_a_vault_blocks::KeySet;
use sapio::contract::abi::object::ObjectMetadata;
use sapio::contract::compiler::compile_policy_leaf;
use sapio::contract::{Compilable, CompilationError, Compiled, Context};
use sapio::template::{OutputAmount, Surplus, Template};
use sapio_base::op_vault::{Recovery, Trigger};
use sapio_base::policy::{PolicyCompiler, ScriptFragment, ScriptPolicy};
use sapio_base::timelocks::RelHeight;
use sapio_base::{Clause, Ctv};

/// A vault's immutable spending terms and a separate default proposal.
#[derive(Clone, Debug)]
pub struct Vault(OpVault);

#[sapio::contract]
impl Vault {
    /// Wrap public terms; compilation validates them in the funding context.
    pub fn new(terms: OpVault) -> Self {
        Self(terms)
    }

    /// Borrow the public terms, including the replaceable preview proposal.
    pub fn terms(&self) -> &OpVault {
        &self.0
    }

    #[policy]
    fn trigger_policy(&self) -> Result<ScriptPolicy, CompilationError> {
        trigger_policy(&self.0)
    }

    #[policy]
    fn recovery_policy(&self, ctx: Context) -> Result<ScriptPolicy, CompilationError> {
        recovery_policy(&self.0, ctx.network)
    }

    /// Choose a withdrawal template and optionally return change to this vault.
    #[action(suggested, guarded_by(Self::trigger_policy), defaults = Self::preview)]
    pub fn trigger(
        &self,
        mut ctx: Context,
        request: WithdrawalProposal,
    ) -> Result<Template, CompilationError> {
        validate(&self.0, &ctx)?;
        request.destination.checked(ctx.network)?;
        let amount = Amount::from_sat(request.withdrawal_sats);
        if amount == Amount::ZERO || amount > ctx.funds() {
            return Err(invalid(
                "withdrawal must be positive and no greater than vault principal",
            ));
        }
        let remainder = ctx.funds() - amount;
        let source = Policies(self.0.clone())
            .compile(ctx.derive_str("source_policy".to_string().into())?)?;
        let pending = Pending {
            terms: self.0.clone(),
            request,
        };
        let withdrawal = pending.withdraw(
            ctx.derive_str("withdrawal_commitment".to_string().into())?
                .with_amount(amount)?,
        )?;
        let predicate = Trigger::new(self.0.delay.blocks, self.0.oracle.xpub).map_err(invalid)?;
        let source_leaf = compile_policy_leaf(&self.trigger_policy()?)?;
        let control = control_for(&source, &source_leaf)?;
        let source_script = source
            .descriptor
            .as_ref()
            .ok_or_else(|| invalid("missing source descriptor"))?
            .script_pubkey();
        let replacement = predicate
            .withdrawal_script(Ctv(withdrawal.hash()))
            .map_err(invalid)?;
        let mut replacement_root = TapNodeHash::from_script(&replacement, LeafVersion::TapScript);
        for sibling in &control.merkle_branch {
            replacement_root = TapNodeHash::from_node_hashes(replacement_root, *sibling);
        }
        let revault = Compiled::from_script(source_script, Amount::from_sat(1), ctx.network)?;
        let mut plan = sponsored(ctx)?;
        plan.output("pending", OutputAmount::Exact(amount), &pending)?;
        if remainder != Amount::ZERO {
            plan.output("revault", OutputAmount::Exact(remainder), &revault)?;
        }
        let mut template = plan.finish()?;
        // Both descriptors were just constructed and validated by the compiler.
        // Compare the committed tree directly; the evaluator verifies the
        // supplied proof against the actual spent output when signing.
        let mut pending_input = bitcoin::psbt::Input::default();
        template.outputs[0]
            .contract
            .descriptor
            .as_ref()
            .ok_or_else(|| invalid("missing pending descriptor"))?
            .update_psbt_input(&mut pending_input)
            .map_err(invalid)?;
        if pending_input.tap_internal_key != Some(control.internal_key)
            || pending_input.tap_merkle_root != Some(replacement_root)
        {
            return Err(invalid(
                "pending policy differs from the evaluator's leaf replacement",
            ));
        }
        let witness = predicate
            .witness(
                Ctv(withdrawal.hash()),
                0,
                (remainder != Amount::ZERO).then_some((1, remainder)),
                &source_leaf,
                &control,
            )
            .map_err(invalid)?;
        template.metadata_map_s2s.label = Some("Trigger withdrawal (external fee sponsor)".into());
        template.metadata_map_s2s.extra.insert(
            "op_vault_witness".into(),
            serde_json::to_value(witness).map_err(invalid)?,
        );
        Ok(template)
    }

    fn preview(&self, ctx: Context) -> Result<Template, CompilationError> {
        self.trigger(ctx, self.0.proposal.clone())
    }

    /// Return all principal to the fixed recovery destination.
    #[action(suggested, default, guarded_by(Self::recovery_policy))]
    pub fn recover(&self, ctx: Context) -> Result<Template, CompilationError> {
        recovery_template(&self.0, ctx)
    }

    #[amount]
    fn ensure_amount(&self, ctx: Context) -> Result<Amount, CompilationError> {
        validate(&self.0, &ctx)?;
        Ok(ctx.funds())
    }

    #[metadata]
    fn metadata(&self, _ctx: Context) -> Result<ObjectMetadata, CompilationError> {
        let mut metadata = ObjectMetadata::default();
        metadata.extra.insert(
            "op_vault_profile".into(),
            serde_json::json!({
                "profile": "bip345-single-vault-v1",
                "oracle": self.0.oracle,
                "delay_blocks": self.0.delay.blocks,
                "fee_sponsor": "native witness v0; no additional P2TR inputs",
                "recovery": self.0.recovery,
            }),
        );
        Ok(metadata)
    }
}

/// The pending output replaces only the trigger leaf; recovery is unchanged.
struct Pending {
    terms: OpVault,
    request: WithdrawalProposal,
}

#[sapio::contract]
impl Pending {
    #[policy]
    fn withdrawal_policy(&self, ctx: Context) -> Result<ScriptPolicy, CompilationError> {
        let template = self.withdraw(ctx)?;
        let predicate =
            Trigger::new(self.terms.delay.blocks, self.terms.oracle.xpub).map_err(invalid)?;
        let policy = ScriptPolicy::And(vec![
            predicate
                .withdrawal_program(Ctv(template.hash()))
                .map_err(invalid)?
                .compile_policy()?,
            Clause::try_from(self.terms.delay.timelock()?)?.into(),
        ]);
        if compile_policy_leaf(&policy)?
            != predicate
                .withdrawal_script(Ctv(template.hash()))
                .map_err(invalid)?
        {
            return Err(invalid(
                "compiler encoding differs from the delayed withdrawal leaf",
            ));
        }
        Ok(policy)
    }

    #[policy]
    fn recovery_policy(&self, ctx: Context) -> Result<ScriptPolicy, CompilationError> {
        recovery_policy(&self.terms, ctx.network)
    }

    #[action(suggested, default, guarded_by(Self::withdrawal_policy))]
    fn withdraw(&self, ctx: Context) -> Result<Template, CompilationError> {
        let amount = ctx.funds();
        let destination =
            Compiled::from_address(self.request.destination.checked(ctx.network)?, amount);
        let mut plan = sponsored(ctx)?;
        plan.require_older(&plan.contract_input(), self.terms.delay.timelock()?.into())?;
        plan.output("withdrawal", OutputAmount::Exact(amount), &destination)?;
        let mut template = plan.finish()?;
        template.metadata_map_s2s.label =
            Some("Withdraw after block delay (external fee sponsor)".into());
        Ok(template)
    }

    #[action(suggested, default, guarded_by(Self::recovery_policy))]
    fn recover(&self, ctx: Context) -> Result<Template, CompilationError> {
        recovery_template(&self.terms, ctx)
    }

    #[amount]
    fn ensure_amount(&self, ctx: Context) -> Result<Amount, CompilationError> {
        validate(&self.terms, &ctx)?;
        Ok(ctx.funds())
    }
}

/// Same two leaves as the source vault, without expanding preview transactions.
struct Policies(OpVault);
#[sapio::contract]
impl Policies {
    #[spend]
    fn trigger(&self) -> Result<ScriptPolicy, CompilationError> {
        trigger_policy(&self.0)
    }
    #[spend]
    fn recover(&self, ctx: Context) -> Result<ScriptPolicy, CompilationError> {
        recovery_policy(&self.0, ctx.network)
    }
}

fn trigger_policy(terms: &OpVault) -> Result<ScriptPolicy, CompilationError> {
    let predicate = Trigger::new(terms.delay.blocks, terms.oracle.xpub).map_err(invalid)?;
    Ok(ScriptPolicy::And(vec![
        authorization_policy(&terms.trigger)?,
        predicate.program().compile_policy()?,
    ]))
}

fn recovery_policy(
    terms: &OpVault,
    network: bitcoin::Network,
) -> Result<ScriptPolicy, CompilationError> {
    let destination = terms.recovery.destination.checked(network)?.script_pubkey();
    let predicate = Recovery::new(&destination, terms.oracle.xpub).map_err(invalid)?;
    Ok(ScriptPolicy::And(vec![
        authorization_policy(&terms.recovery.authorization)?,
        predicate.program().compile_policy()?,
    ]))
}

fn authorization_policy(keys: &KeySet) -> Result<ScriptPolicy, CompilationError> {
    keys.validate()?;
    if keys.keys.len() != 1 {
        return keys.policy();
    }
    // Keep authorization separate from the program's native signing clause,
    // avoiding a fresh two-key optimizer search during every policy replay.
    // n:pk retains canonical bytes after Sapio appends OP_VERIFY, so native
    // planners recover the exact TapLeaf hash used by the signature.
    Ok(ScriptFragment::new(
        bitcoin::script::Builder::new()
            .push_x_only_key(&keys.keys[0])
            .push_opcode(bitcoin::opcodes::all::OP_CHECKSIG)
            .push_opcode(bitcoin::opcodes::all::OP_0NOTEQUAL)
            .into_script(),
    )?
    .into())
}

fn recovery_template(terms: &OpVault, ctx: Context) -> Result<Template, CompilationError> {
    let amount = ctx.funds();
    let destination =
        Compiled::from_address(terms.recovery.destination.checked(ctx.network)?, amount);
    let mut plan = sponsored(ctx)?;
    plan.output("recovery", OutputAmount::Exact(amount), &destination)?;
    let mut template = plan.finish()?;
    template.metadata_map_s2s.label = Some("Recover principal (external fee sponsor)".into());
    template
        .metadata_map_s2s
        .extra
        .insert("op_vault_witness".into(), serde_json::json!([0, 0, 0, 0]));
    Ok(template)
}

fn sponsored<'a>(ctx: Context) -> Result<sapio::template::TemplatePlan<'a>, CompilationError> {
    let mut plan = ctx.template_plan();
    let sponsor = plan.input("fee_sponsor", Amount::ZERO)?;
    plan.require_older(&sponsor, RelHeight::from(0).into())?;
    // This is a local binding cap, not a covenant predicate. All vault principal
    // is allocated to outputs; only the separately selected sponsor pays fees.
    plan.surplus(Surplus::Fees {
        maximum: Amount::from_sat(100_000),
    });
    Ok(plan)
}

fn validate(terms: &OpVault, ctx: &Context) -> Result<(), CompilationError> {
    terms.trigger.validate()?;
    terms.recovery.validate(ctx.network)?;
    terms.delay.validate()?;
    terms.oracle.validate(ctx.network)?;
    if ctx.funds() == Amount::ZERO || ctx.funds() > Amount::MAX_MONEY {
        return Err(invalid(
            "vault principal must be positive and within MAX_MONEY",
        ));
    }
    Ok(())
}

fn control_for(source: &Compiled, leaf: &ScriptBuf) -> Result<ControlBlock, CompilationError> {
    let mut input = bitcoin::psbt::Input::default();
    source
        .descriptor
        .as_ref()
        .ok_or_else(|| invalid("missing vault descriptor"))?
        .update_psbt_input(&mut input)
        .map_err(invalid)?;
    input
        .tap_scripts
        .into_iter()
        .find_map(|(control, (script, version))| {
            (script == *leaf && version == LeafVersion::TapScript).then_some(control)
        })
        .ok_or_else(|| invalid("trigger leaf missing from source descriptor"))
}
