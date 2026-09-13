//! TemplateHash and CSFS authorization with either a physical or proven internal key.
//!
//! The public contract fixes its destinations, fee and authorization program.
//! Continuations propose payment amounts without changing the spending policy.

use bitcoin::bip32::Xpub;
use bitcoin::{Address, Amount, XOnlyPublicKey};
use sapio::contract::*;
use sapio::template::{OutputAmount, Template};
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

#[sapio::contract]
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

    #[policy]
    fn authorize(&self) -> EmulatedProgram {
        self.program.clone()
    }

    #[spend]
    fn key_path(&self) -> Clause {
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

    /// Construct an authorized payment candidate with explicitly allocated change.
    #[action(suggested, guarded_by(Self::authorize))]
    pub fn pay(&self, ctx: Context, amount: u64) -> Result<Template, CompilationError> {
        let recipient = Compiled::from_address(self.recipient.clone(), Amount::ZERO);
        let change_address = Compiled::from_address(self.change.clone(), Amount::ZERO);
        let mut plan = ctx.template_plan();
        plan.output(
            "recipient",
            OutputAmount::Exact(Amount::from_sat(amount)),
            &recipient,
        )?;
        plan.output("change", OutputAmount::Remainder, &change_address)?;
        plan.reserve_fees(self.fee);
        plan.finish().map_err(Into::into)
    }

    #[internal_key]
    fn pinned_internal_key(
        &self,
        _ctx: &Context,
    ) -> Result<Option<XOnlyPublicKey>, CompilationError> {
        Ok(Some(self.internal_key()))
    }
}
