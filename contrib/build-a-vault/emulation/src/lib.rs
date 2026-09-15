//! Public OP_VAULT terms and Studio ports. Contracts live in `contracts`;
//! signing, fee sponsorship and fixtures belong to runners and tests.

#![deny(missing_docs)]

pub mod contracts;

use bitcoin::bip32::Xpub;
use build_a_vault_blocks::{AddressTarget, KeySet, RecoveryRule, RelativeDelay};
use sapio::contract::{Compilable, CompilationError, Compiled, Context};
use sapio_wasm_plugin::client::plugin::Callable;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) fn invalid(message: impl ToString) -> CompilationError {
    CompilationError::Custom(message.to_string().into())
}

/// Public oracle identity. Its private root never belongs in a Studio patch.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OracleRoot {
    /// BIP32 public root used to derive every evaluator signing key.
    pub xpub: Xpub,
}

/// Select the public root of the explicit covenant emulator.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmulationOracle {
    /// Public extended key, obtained from the intended signing service.
    pub xpub: Xpub,
}

impl Callable for EmulationOracle {
    type Output = OracleRoot;
    fn call(&self, ctx: Context) -> Result<Self::Output, CompilationError> {
        let root = OracleRoot { xpub: self.xpub };
        root.validate(ctx.network)?;
        Ok(root)
    }
}

impl OracleRoot {
    /// Check the network and that the root can derive an evaluator key.
    pub fn validate(&self, network: bitcoin::Network) -> Result<(), CompilationError> {
        if self.xpub.network != network.into() {
            return Err(invalid("emulator root must match the compilation network"));
        }
        sapio_base::op_vault::Trigger::new(1, self.xpub).map_err(invalid)?;
        Ok(())
    }
}

/// A proposed withdrawal; changing it does not change the funding address.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WithdrawalProposal {
    /// Destination selected at trigger time, committed by the pending CTV leaf.
    pub destination: AddressTarget,
    /// Principal moved into the waiting period; the remainder is revaulted.
    pub withdrawal_sats: u64,
}

/// Compile a single-input OP_VAULT emulation with a suggested withdrawal.
///
/// The source and pending coins retain the same fixed recovery leaf. Trigger
/// replaces the trigger leaf with a CSV plus emulated CTV leaf. Every movement
/// preserves principal and declares a separate witness-v0 fee sponsor input.
/// Compilation uses the normal host fuel budget; combining large trigger and
/// recovery quorums may exceed it even when both key sets are individually valid.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpVault {
    /// Signatures that may choose a withdrawal template and trigger it.
    pub trigger: KeySet,
    /// Authorized fixed recovery before or after triggering.
    pub recovery: RecoveryRule,
    /// Waiting period beginning when the pending output confirms.
    pub delay: RelativeDelay,
    /// Public root for OP_VAULT, recovery and final CTV emulation.
    pub oracle: OracleRoot,
    /// Preview transaction, separate from the fixed funding policy.
    pub proposal: WithdrawalProposal,
}

impl Callable for OpVault {
    type Output = Compiled;
    fn call(&self, ctx: Context) -> Result<Compiled, CompilationError> {
        contracts::Vault::new(self.clone()).compile(ctx)
    }
}
