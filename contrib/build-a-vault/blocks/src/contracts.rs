//! Native contracts behind the terminal WASM modules.

use crate::{invalid, DelayedWallet, FixedVault, RecoveryRule, ReleaseRule};
use bitcoin::Amount;
use sapio::contract::{CompilationError, Compiled, Context};
use sapio::template::{OutputAmount, Template};
use sapio_base::policy::ScriptPolicy;

pub(crate) struct VaultContract(pub FixedVault);

#[sapio::contract]
impl VaultContract {
    #[policy]
    fn trigger_authorization(&self) -> Result<ScriptPolicy, CompilationError> {
        self.0.trigger.policy()
    }

    #[policy]
    fn recovery_authorization(&self) -> Result<ScriptPolicy, CompilationError> {
        self.0.recovery.authorization.policy()
    }

    #[action(committed, guarded_by(Self::trigger_authorization))]
    fn trigger(&self, ctx: Context) -> Result<Template, CompilationError> {
        let pending = PendingContract {
            release: self.0.release.clone(),
            recovery: self.0.recovery.clone(),
            fee_sats: self.0.fee_sats,
        };
        let mut plan = ctx.template_plan();
        plan.output("pending", OutputAmount::Remainder, &pending)?;
        plan.reserve_fees(Amount::from_sat(self.0.fee_sats));
        Ok(plan.finish()?)
    }

    #[action(committed, guarded_by(Self::recovery_authorization))]
    fn recover(&self, ctx: Context) -> Result<Template, CompilationError> {
        recover(ctx, &self.0.recovery, self.0.fee_sats)
    }

    #[amount]
    fn validate(&self, ctx: Context) -> Result<Amount, CompilationError> {
        self.0.trigger.validate()?;
        self.0.release.validate(ctx.network)?;
        self.0.recovery.validate(ctx.network)?;
        require_budget(&ctx, self.0.fee_sats, 2)?;
        Ok(ctx.funds())
    }
}

struct PendingContract {
    release: ReleaseRule,
    recovery: RecoveryRule,
    fee_sats: u64,
}

#[sapio::contract]
impl PendingContract {
    #[policy]
    fn release_authorization(&self) -> Result<ScriptPolicy, CompilationError> {
        self.release.authorization.policy()
    }

    #[policy]
    fn recovery_authorization(&self) -> Result<ScriptPolicy, CompilationError> {
        self.recovery.authorization.policy()
    }

    #[action(committed, guarded_by(Self::release_authorization))]
    fn release(&self, ctx: Context) -> Result<Template, CompilationError> {
        let destination = Compiled::from_address(
            self.release.destination.checked(ctx.network)?,
            Amount::from_sat(1),
        );
        let mut plan = ctx.template_plan();
        let input = plan.contract_input();
        plan.require_older(&input, self.release.delay.timelock()?.into())?;
        plan.output("release", OutputAmount::Remainder, &destination)?;
        plan.reserve_fees(Amount::from_sat(self.fee_sats));
        Ok(plan.finish()?)
    }

    #[action(committed, guarded_by(Self::recovery_authorization))]
    fn recover(&self, ctx: Context) -> Result<Template, CompilationError> {
        recover(ctx, &self.recovery, self.fee_sats)
    }

    #[amount]
    fn validate(&self, ctx: Context) -> Result<Amount, CompilationError> {
        self.release.validate(ctx.network)?;
        self.recovery.validate(ctx.network)?;
        require_budget(&ctx, self.fee_sats, 1)?;
        Ok(ctx.funds())
    }
}

fn recover(ctx: Context, rule: &RecoveryRule, fee_sats: u64) -> Result<Template, CompilationError> {
    let destination =
        Compiled::from_address(rule.destination.checked(ctx.network)?, Amount::from_sat(1));
    let mut plan = ctx.template_plan();
    plan.output("recovery", OutputAmount::Remainder, &destination)?;
    plan.reserve_fees(Amount::from_sat(fee_sats));
    Ok(plan.finish()?)
}

fn require_budget(ctx: &Context, fee_sats: u64, steps: u64) -> Result<(), CompilationError> {
    if ctx.funds() > Amount::MAX_MONEY {
        return Err(invalid(
            "vault funding exceeds Bitcoin's maximum money amount",
        ));
    }
    let fees = fee_sats
        .checked_mul(steps)
        .ok_or_else(|| invalid("vault fee budget overflow"))?;
    if ctx.funds().to_sat() <= fees {
        return Err(invalid(
            "vault funding must leave positive outputs after every fee",
        ));
    }
    Ok(())
}

pub(crate) struct WalletContract(pub DelayedWallet);

#[sapio::contract]
impl WalletContract {
    #[internal_key]
    fn recovery_key(
        &self,
        _ctx: &Context,
    ) -> Result<Option<bitcoin::XOnlyPublicKey>, CompilationError> {
        self.0.recovery.validate()?;
        Ok((self.0.recovery.keys.len() == 1).then(|| self.0.recovery.keys[0]))
    }

    #[spend]
    fn delayed_hot(&self) -> Result<ScriptPolicy, CompilationError> {
        self.0.hot.delayed_policy(self.0.delay)
    }

    #[spend]
    fn recovery(&self) -> Result<ScriptPolicy, CompilationError> {
        self.0.recovery.policy()
    }

    #[amount]
    fn validate(&self, ctx: Context) -> Result<Amount, CompilationError> {
        self.0.hot.validate()?;
        self.0.recovery.validate()?;
        self.0.delay.validate()?;
        if ctx.funds() == Amount::ZERO || ctx.funds() > Amount::MAX_MONEY {
            return Err(invalid(
                "wallet funding must be positive and at most MAX_MONEY",
            ));
        }
        Ok(ctx.funds())
    }
}
