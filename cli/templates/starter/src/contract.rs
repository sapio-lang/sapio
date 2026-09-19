//! A payment predicate and the transactions its typed action proposes.

use bitcoin::{Address, Amount};
use sapio::contract::{CompilationError, Compiled, Context};
use sapio::template::{OutputAmount, Template};
use sapio_base::program::{EmulatedProgram, EvaluatorId, ProgramError, ProgramInstance};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A candidate may pay more than the minimum fixed by the contract.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct PaymentRequest {
    pub amount_sats: u64,
}

/// The oracle must enforce the committed recipient and minimum honestly.
pub struct Payment {
    recipient: Address,
    change: Address,
    authorization: EmulatedProgram,
}

#[sapio::contract]
impl Payment {
    pub fn new(
        recipient: Address,
        change: Address,
        minimum: Amount,
        evaluator: EvaluatorId,
        oracle: bitcoin::bip32::Xpub,
    ) -> Result<Self, ProgramError> {
        let script = recipient.script_pubkey();
        // The distributed pay-at-least/v1 evaluator defines this exact codec.
        let mut parameters = minimum.to_sat().to_le_bytes().to_vec();
        parameters.extend_from_slice(&(script.len() as u32).to_le_bytes());
        parameters.extend_from_slice(script.as_bytes());
        let instance = ProgramInstance::new(evaluator, b"pay-at-least/v1".to_vec(), parameters)?;
        Ok(Self {
            recipient,
            change,
            authorization: EmulatedProgram::new(instance, oracle)?,
        })
    }

    #[policy]
    fn authorize(&self) -> EmulatedProgram {
        self.authorization.clone()
    }

    /// Construct a proposal; only the fixed policy can authorize its spend.
    #[action(suggested, guarded_by(Self::authorize))]
    pub fn pay(&self, ctx: Context, request: PaymentRequest) -> Result<Template, CompilationError> {
        let recipient = Compiled::from_address(self.recipient.clone(), Amount::ZERO);
        let change = Compiled::from_address(self.change.clone(), Amount::ZERO);
        let mut plan = ctx.template_plan();
        plan.output(
            "recipient",
            OutputAmount::Exact(Amount::from_sat(request.amount_sats)),
            &recipient,
        )?;
        plan.output("change", OutputAmount::Remainder, &change)?;
        plan.reserve_fees(Amount::from_sat(500));
        plan.require_feerate(
            bitcoin::FeeRate::from_sat_per_vb(1).expect("valid constant fee rate"),
        );
        Ok(plan.finish()?)
    }
}
