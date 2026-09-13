//! A custom policy language composed with native guards and a Sapio covenant.

use bitcoin::blockdata::{opcodes::all, script::Builder};
use bitcoin::{Amount, XOnlyPublicKey};
use sapio::contract::{CompilationError, Context, Contract};
use sapio::template::{OutputAmount, Template};
use sapio::{declare, guard, then};
use sapio_base::policy::{PolicyCompiler, PolicyError, ScriptFragment, ScriptPolicy};
use sapio_base::Clause;
use schemars::JsonSchema;
use serde::Deserialize;

#[cfg(target_arch = "wasm32")]
use sapio_wasm_plugin::{optional_logo, REGISTER};

/// An external language whose arithmetic preserves a signature's truth value.
pub struct ArithmeticSigner(XOnlyPublicKey);

impl PolicyCompiler for ArithmeticSigner {
    fn compile_policy(&self) -> Result<ScriptPolicy, PolicyError> {
        ScriptFragment::new(
            Builder::new()
                .push_slice(&self.0.serialize())
                .push_opcode(all::OP_CHECKSIG)
                .push_opcode(all::OP_1ADD)
                .push_int(2)
                .push_opcode(all::OP_NUMEQUAL)
                .into_script(),
        )
        .map(Into::into)
    }
}

/// Owner and co-signer authorize a fixed payment, reserving 1,000 sats as fees.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CustomPolicyPayment {
    /// Signer expressed through the custom arithmetic language.
    owner: XOnlyPublicKey,
    /// Native Miniscript signer, also receiving the payment.
    co_signer: XOnlyPublicKey,
}

impl CustomPolicyPayment {
    #[guard(policy)]
    fn custom_owner(self, _ctx: Context) -> ArithmeticSigner {
        ArithmeticSigner(self.owner)
    }

    #[guard]
    fn native_co_signer(self, _ctx: Context) {
        Clause::Key(self.co_signer)
    }

    #[then(guarded_by = "[Self::custom_owner, Self::native_co_signer]")]
    fn pay(self, ctx: Context) -> Result<Template, CompilationError> {
        let fees = Amount::from_sat(1_000);
        let payment = ctx
            .funds()
            .checked_sub(fees)
            .ok_or(CompilationError::OutOfFunds)?;
        if payment == Amount::ZERO {
            return Err(CompilationError::OutOfFunds);
        }
        let mut plan = ctx.template_plan();
        plan.output("payment", OutputAmount::Exact(payment), &self.co_signer)?;
        plan.reserve_fees(fees);
        Ok(plan.finish()?)
    }
}

impl Contract for CustomPolicyPayment {
    declare! {actions, Self::pay}
}

#[cfg(target_arch = "wasm32")]
REGISTER![CustomPolicyPayment];

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
    use sapio::contract::abi::object::SupportedDescriptors;
    use sapio::contract::Compilable;
    use sapio_base::covenant::LoweringPlan;
    use std::sync::Arc;

    #[test]
    fn custom_and_native_guards_compile_a_checked_payment() {
        let key = |byte| {
            Keypair::from_secret_key(
                &Secp256k1::new(),
                &SecretKey::from_slice(&[byte; 32]).unwrap(),
            )
            .x_only_public_key()
            .0
        };
        let contract = CustomPolicyPayment {
            owner: key(1),
            co_signer: key(2),
        };
        for amount in [1_000, 10_000] {
            let result = contract.compile(Context::new(
                bitcoin::Network::Regtest,
                Amount::from_sat(amount),
                LoweringPlan::Native,
                "custom_policy".try_into().unwrap(),
                Arc::new(Default::default()),
                None,
            ));
            if amount == 1_000 {
                assert!(matches!(result, Err(CompilationError::OutOfFunds)));
            } else {
                let compiled = result.unwrap();
                compiled.validate().unwrap();
                assert!(matches!(
                    compiled.descriptor,
                    Some(SupportedDescriptors::Taproot(_))
                ));
                assert_eq!(
                    compiled.ctv_to_tx.values().next().unwrap().tx.output[0]
                        .value
                        .to_sat(),
                    9_000
                );
                let funding = compiled
                    .ctv_to_tx
                    .values()
                    .next()
                    .unwrap()
                    .funding_constraints
                    .as_ref()
                    .unwrap();
                assert_eq!(funding.inputs[0].minimum.to_sat(), amount);
                assert_eq!(funding.outputs, ["payment"]);
                assert_eq!(funding.maximum_fee.to_sat(), 1_000);
            }
        }
    }
}
