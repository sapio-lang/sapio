//! Small, schema-compatible custody building blocks for Sapio Studio.
//!
//! Intermediate values describe public terms. Only the terminal blocks compile
//! contracts; none of the intermediate values confer signing authority.

#![deny(missing_docs)]

mod contracts;

use bitcoin::address::NetworkUnchecked;
use bitcoin::{Address, Network, XOnlyPublicKey};
use sapio::contract::{Compilable, CompilationError, Compiled, Context};
use sapio_base::miniscript::{Miniscript, RelLockTime, Tap, Terminal, Threshold};
use sapio_base::policy::{ScriptFragment, ScriptPolicy};
use sapio_base::timelocks::RelHeight;
use sapio_base::Clause;
use sapio_wasm_plugin::client::plugin::Callable;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::Arc;

pub(crate) fn invalid(message: &str) -> CompilationError {
    CompilationError::Custom(message.into())
}

/// A threshold over distinct public keys, shared by every authorization port.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeySet {
    /// Number of distinct signatures required.
    #[schemars(range(min = 1, max = 16))]
    pub threshold: u8,
    /// One through sixteen x-only public keys, in policy order.
    #[schemars(length(min = 1, max = 16))]
    pub keys: Vec<XOnlyPublicKey>,
}

impl KeySet {
    /// Check authorization invariants even for values supplied directly as JSON.
    pub fn validate(&self) -> Result<(), CompilationError> {
        if self.keys.is_empty() || self.keys.len() > 16 {
            return Err(invalid("authorization needs between one and sixteen keys"));
        }
        if self.threshold == 0 || usize::from(self.threshold) > self.keys.len() {
            return Err(invalid(
                "authorization threshold must be within its key count",
            ));
        }
        if self.keys.iter().copied().collect::<BTreeSet<_>>().len() != self.keys.len() {
            return Err(invalid("authorization keys must be distinct"));
        }
        Ok(())
    }

    /// Express the authorization as a concrete policy for semantic inspection.
    /// Use [`Self::policy`] when compiling a block to avoid threshold search.
    pub fn clause(&self) -> Result<Clause, CompilationError> {
        self.validate()?;
        if self.keys.len() == 1 {
            return Ok(Clause::Key(self.keys[0]));
        }
        Ok(Clause::Thresh(
            Threshold::new(
                usize::from(self.threshold),
                self.keys
                    .iter()
                    .map(|key| Clause::Key(*key).into())
                    .collect(),
            )
            .map_err(|_| invalid("invalid authorization threshold"))?,
        ))
    }

    /// Encode a standard CHECKSIGADD quorum without running a policy search.
    ///
    /// The fragment remains ordinary Miniscript: the spend planner recovers
    /// its signature slots and satisfaction bounds from the encoded script.
    /// Single signers retain a bare native clause for internal-key selection.
    pub fn policy(&self) -> Result<ScriptPolicy, CompilationError> {
        self.validate()?;
        if self.keys.len() == 1 {
            return Ok(Clause::Key(self.keys[0]).into());
        }
        // Sapio appends OP_VERIFY between fragments. Normalizing this already
        // boolean result prevents NUMEQUAL + VERIFY from re-encoding as the
        // shorter NUMEQUALVERIFY. Native planners must recover the exact same
        // leaf bytes, since signatures commit to their TapLeaf hash.
        let boundary = Miniscript::from_ast(Terminal::ZeroNotEqual(Arc::new(self.multisig()?)))
            .map_err(|_| invalid("cannot normalize canonical quorum boundary"))?;
        Ok(ScriptFragment::new(boundary.encode())?.into())
    }

    /// Require a standard quorum and a positive relative delay in one fragment.
    ///
    /// Keeping the conjunction together lets the native planner recognize the
    /// whole signature-and-delay policy without an unconstrained timelock run.
    pub fn delayed_policy(&self, delay: RelativeDelay) -> Result<ScriptPolicy, CompilationError> {
        self.validate()?;
        delay.validate()?;
        if self.keys.len() == 1 {
            return Ok(Clause::And(vec![
                Clause::Key(self.keys[0]).into(),
                Clause::try_from(delay.timelock()?)?.into(),
            ])
            .into());
        }
        let authorization = Miniscript::from_ast(Terminal::Verify(Arc::new(self.multisig()?)))
            .map_err(|_| invalid("cannot verify canonical quorum"))?;
        let age = Miniscript::from_ast(Terminal::Older(RelLockTime::from_height(delay.blocks)))
            .map_err(|_| invalid("cannot encode canonical block delay"))?;
        let script = Miniscript::<XOnlyPublicKey, Tap>::from_ast(Terminal::AndV(
            Arc::new(authorization),
            Arc::new(age),
        ))
        .map_err(|_| invalid("cannot combine canonical quorum and delay"))?;
        Ok(ScriptFragment::new(script.encode())?.into())
    }

    fn multisig(&self) -> Result<Miniscript<XOnlyPublicKey, Tap>, CompilationError> {
        let keys = Threshold::new(usize::from(self.threshold), self.keys.clone())
            .map_err(|_| invalid("invalid canonical quorum threshold"))?;
        Miniscript::from_ast(Terminal::MultiA(keys))
            .map_err(|_| invalid("cannot encode canonical quorum"))
    }
}

/// A positive relative block delay; it starts when the encumbered coin confirms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RelativeDelay {
    /// Confirmation age in blocks, from one through 65535.
    #[schemars(range(min = 1, max = 65535))]
    pub blocks: u16,
}

impl RelativeDelay {
    /// Reject a zero-length waiting period.
    pub fn validate(&self) -> Result<(), CompilationError> {
        if self.blocks == 0 {
            return Err(invalid("a block delay must be positive"));
        }
        Ok(())
    }

    /// Produce a typed sequence constraint after validating the delay.
    pub fn timelock(&self) -> Result<RelHeight, CompilationError> {
        self.validate()?;
        Ok(RelHeight::from(self.blocks))
    }
}

/// A fixed payment destination, checked against each consumer's compilation network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddressTarget {
    /// The address that receives this route's output.
    pub address: Address<NetworkUnchecked>,
}

impl AddressTarget {
    /// Resolve the address for the explicit compilation network.
    pub fn checked(&self, network: Network) -> Result<Address, CompilationError> {
        Ok(self.address.clone().require_network(network)?)
    }
}

/// A fixed recovery destination and the authorization needed to send funds there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecoveryRule {
    /// Signatures needed for the recovery transaction.
    pub authorization: KeySet,
    /// Destination fixed by the compiled contract.
    pub destination: AddressTarget,
}

impl RecoveryRule {
    /// Check nested terms at the consuming module boundary.
    pub fn validate(&self, network: Network) -> Result<(), CompilationError> {
        self.authorization.validate()?;
        self.destination.checked(network)?;
        Ok(())
    }
}

/// A delayed, authorized release to one fixed destination.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReleaseRule {
    /// Signatures needed after the waiting period.
    pub authorization: KeySet,
    /// Minimum age of the pending withdrawal output.
    pub delay: RelativeDelay,
    /// Destination fixed when the vault is compiled.
    pub destination: AddressTarget,
}

impl ReleaseRule {
    /// Check nested terms at the consuming module boundary.
    pub fn validate(&self, network: Network) -> Result<(), CompilationError> {
        self.authorization.validate()?;
        self.delay.validate()?;
        self.destination.checked(network)?;
        Ok(())
    }
}

/// Select one public key for an authorization socket.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Signer {
    /// The signer public key; private keys never belong in patch arguments.
    pub key: XOnlyPublicKey,
}

impl Callable for Signer {
    type Output = KeySet;
    fn call(&self, _ctx: Context) -> Result<KeySet, CompilationError> {
        Ok(KeySet {
            threshold: 1,
            keys: vec![self.key],
        })
    }
}

/// Select a threshold of distinct signers for an authorization socket.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Quorum {
    /// Number of signatures required.
    #[schemars(range(min = 1, max = 16))]
    pub threshold: u8,
    /// Distinct public keys; order is retained in the public terms.
    #[schemars(length(min = 1, max = 16))]
    pub keys: Vec<XOnlyPublicKey>,
}

impl Callable for Quorum {
    type Output = KeySet;
    fn call(&self, _ctx: Context) -> Result<KeySet, CompilationError> {
        let terms = KeySet {
            threshold: self.threshold,
            keys: self.keys.clone(),
        };
        terms.validate()?;
        Ok(terms)
    }
}

/// Choose a positive confirmation delay in blocks.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlockDelay {
    /// Number of blocks the encumbered output must age before use.
    #[schemars(range(min = 1, max = 65535))]
    pub blocks: u16,
}

impl Callable for BlockDelay {
    type Output = RelativeDelay;
    fn call(&self, _ctx: Context) -> Result<RelativeDelay, CompilationError> {
        let terms = RelativeDelay {
            blocks: self.blocks,
        };
        terms.validate()?;
        Ok(terms)
    }
}

/// Choose a destination on the patch's compilation network.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Destination {
    /// A Bitcoin address whose network must match the explicit context.
    pub address: Address<NetworkUnchecked>,
}

impl Callable for Destination {
    type Output = AddressTarget;
    fn call(&self, ctx: Context) -> Result<AddressTarget, CompilationError> {
        let terms = AddressTarget {
            address: self.address.clone(),
        };
        terms.checked(ctx.network)?;
        Ok(terms)
    }
}

/// Combine authorization and a fixed recovery destination.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Recovery {
    /// Required recovery signatures.
    pub authorization: KeySet,
    /// Recovery destination.
    pub destination: AddressTarget,
}

impl Callable for Recovery {
    type Output = RecoveryRule;
    fn call(&self, ctx: Context) -> Result<RecoveryRule, CompilationError> {
        let terms = RecoveryRule {
            authorization: self.authorization.clone(),
            destination: self.destination.clone(),
        };
        terms.validate(ctx.network)?;
        Ok(terms)
    }
}

/// Combine authorization, a waiting period and a fixed release destination.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Release {
    /// Required release signatures.
    pub authorization: KeySet,
    /// Pending-output confirmation delay.
    pub delay: RelativeDelay,
    /// Fixed release destination.
    pub destination: AddressTarget,
}

impl Callable for Release {
    type Output = ReleaseRule;
    fn call(&self, ctx: Context) -> Result<ReleaseRule, CompilationError> {
        let terms = ReleaseRule {
            authorization: self.authorization.clone(),
            delay: self.delay,
            destination: self.destination.clone(),
        };
        terms.validate(ctx.network)?;
        Ok(terms)
    }
}

/// Compile a two-stage vault whose trigger, release and recovery all fix outputs.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FixedVault {
    /// Signatures needed to move funds into the pending withdrawal output.
    pub trigger: KeySet,
    /// Authorized, delayed payment from the pending output.
    pub release: ReleaseRule,
    /// Authorized fixed recovery available before and after triggering.
    pub recovery: RecoveryRule,
    /// Satoshis reserved in each transaction: trigger, release or recovery.
    pub fee_sats: u64,
}

impl Callable for FixedVault {
    type Output = Compiled;
    fn call(&self, ctx: Context) -> Result<Compiled, CompilationError> {
        contracts::VaultContract(self.clone()).compile(ctx)
    }
}

/// Compile a wallet with an immediate recovery path and a delayed hot-key path.
///
/// Both paths permit arbitrary destinations. This is a delayed wallet, not a
/// covenant vault: its delay starts when the wallet output itself confirms.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DelayedWallet {
    /// Signatures that can spend anywhere after the delay.
    pub hot: KeySet,
    /// Minimum age of the wallet output for its hot-key path.
    pub delay: RelativeDelay,
    /// Signatures that can immediately spend anywhere.
    pub recovery: KeySet,
}

impl Callable for DelayedWallet {
    type Output = Compiled;
    fn call(&self, ctx: Context) -> Result<Compiled, CompilationError> {
        contracts::WalletContract(self.clone()).compile(ctx)
    }
}

#[cfg(test)]
mod tests;
