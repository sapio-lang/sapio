//! TemplateHash and CSFS authorization with either a physical or proven internal key.
//!
//! The public contract fixes its destinations, fee and authorization program.
//! Continuations propose payment amounts without changing the spending policy.

use bitcoin::bip32::Xpub;
use bitcoin::{Address, Amount, XOnlyPublicKey};
use sapio::contract::*;
use sapio::*;
use sapio_base::fragments::{template_signed_by, TemplateKey};
use sapio_base::program::{EmulatedProgram, ProgramError};
use sapio_base::Clause;

/// Which key authorizes the proposed transaction template.
#[derive(Clone, Copy, Debug)]
pub enum Authorization {
    /// Pin a participant as the physical internal key, with a script-path program.
    InternalKey(XOnlyPublicKey),
    /// Use the program as the internal key and prove its relation to the root.
    KnownTweak,
}

/// Payments authorized by a TemplateHash signature under the selected key.
pub struct FragmentContract {
    authorization: Authorization,
    program: EmulatedProgram,
    recipient: Address,
    change: Address,
    fee: Amount,
}

impl FragmentContract {
    /// Bind the public spending terms and the exact emulated fragment program.
    pub fn new(
        authorization: Authorization,
        oracle_root: Xpub,
        recipient: Address,
        change: Address,
        fee: Amount,
    ) -> Result<Self, ProgramError> {
        let key = match authorization {
            Authorization::InternalKey(_) => TemplateKey::InternalKey,
            Authorization::KnownTweak => TemplateKey::KnownTweak,
        };
        Ok(Self {
            authorization,
            program: template_signed_by(key, oracle_root)?,
            recipient,
            change,
            fee,
        })
    }

    #[guard(policy, cached)]
    fn authorize(self) -> EmulatedProgram {
        self.program.clone()
    }

    #[guard(cached)]
    fn key_path(self) {
        Clause::Key(self.internal_key())
    }

    fn internal_key(&self) -> XOnlyPublicKey {
        match self.authorization {
            Authorization::InternalKey(key) => key,
            Authorization::KnownTweak => self
                .program
                .derive_public_key()
                .expect("validated public program root"),
        }
    }

    #[continuation(guarded_by = "[Self::authorize]", web_api)]
    fn pay(self, ctx: Context, amount: Option<u64>) {
        let Some(amount) = amount else { return empty() };
        let amount = Amount::from_sat(amount);
        let change = ctx
            .funds()
            .checked_sub(self.fee)
            .and_then(|available| available.checked_sub(amount))
            .ok_or(CompilationError::OutOfFunds)?;
        let recipient = Compiled::from_address(self.recipient.clone(), Amount::ZERO);
        let change_address = Compiled::from_address(self.change.clone(), Amount::ZERO);
        ctx.template()
            .add_output(amount, &recipient, None)?
            .add_output(change, &change_address, None)?
            .add_fees(self.fee)?
            .into()
    }
}

impl Contract for FragmentContract {
    declare! {finish, Self::key_path}
    declare! {actions, Self::pay}

    fn pinned_internal_key(
        &self,
        _ctx: &Context,
    ) -> Result<Option<XOnlyPublicKey>, CompilationError> {
        Ok(Some(self.internal_key()))
    }
}
