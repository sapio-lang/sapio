//! LayerTree state outputs whose actual covenant scripts execute in the oracle.
//!
//! The native Taproot leaves contain the interpreter's program keys. Their
//! physical Merkle root is the `m` consumed by the unchanged MID scripts. The
//! internal key is `H + H_LayerTree/state(root)*G`, so the script's TREE_SPK
//! calculation binds directly to Bitcoin outputs, including successor states.
//! Every leaf remains a script path with a public NUMS opening.

use bitcoin::bip32::Xpub;
use bitcoin::{Amount, ScriptBuf};
use sapio::contract::actions::Guard;
use sapio::contract::{Compilable, CompilationError, Compiled, Context, Contract};
use sapio_base::nums::NumsInternalKey;
use sapio_base::policy::ScriptPolicy;
use sapio_base::program::EmulatedProgram;

pub mod script;
pub mod tree;
pub mod witness;

pub use script::{Config, Transition};
pub use tree::{Leaf, Node, Proof, Tree};

fn invalid(error: impl std::fmt::Display) -> CompilationError {
    CompilationError::Custom(error.to_string().into())
}

/// Fixed script-set terms shared by every state of one LayerTree instance.
///
/// The state root and amount belong to each output, rather than to interpreter
/// parameters. This keeps the physical script-tree commitment stable while
/// MID Script checks a replacement root and its exact summed principal.
#[derive(Clone, Debug)]
pub struct LayerTree {
    config: Config,
    oracle_root: Xpub,
}

impl LayerTree {
    /// Validate the supported dense MID script set and oracle derivations.
    pub fn new(config: Config, oracle_root: Xpub) -> Result<Self, CompilationError> {
        config.validate().map_err(invalid)?;
        let contract = Self {
            config,
            oracle_root,
        };
        for transition in contract.config.transitions() {
            contract.program(transition)?;
        }
        Ok(contract)
    }

    /// The immutable upstream MID compilation parameters.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Public root authorized to evaluate each exact Script instance.
    pub fn oracle_root(&self) -> Xpub {
        self.oracle_root
    }

    /// The complete MID interpreter program for one enabled transition.
    pub fn program(&self, transition: Transition) -> Result<EmulatedProgram, CompilationError> {
        if !self.config.transitions().contains(&transition) {
            return Err(invalid("transition is not in this LayerTree script set"));
        }
        let script = script::script(&self.config, transition).map_err(invalid)?;
        if script.len() > 10_000 {
            return Err(invalid(
                "MID Script exceeds the interpreter's 10,000-byte limit",
            ));
        }
        sapio_base::bitcoin_script::emulated_mid_script(&script, self.oracle_root).map_err(invalid)
    }

    /// Compile a funded state from its complete off-chain Merkle-sum tree.
    pub fn compile_state(&self, ctx: Context, tree: &Tree) -> Result<Compiled, CompilationError> {
        if tree.depth() != self.config.depth {
            return Err(invalid(
                "tree depth does not match the committed script set",
            ));
        }
        self.compile_root(ctx, tree.root())
    }

    /// Compile a funded state from its authenticated root and summed amount.
    ///
    /// This helper does not assert possession of every leaf or proof. The
    /// covenant verifies supplied transition proofs when requesting a spend.
    pub fn compile_root(&self, ctx: Context, root: Node) -> Result<Compiled, CompilationError> {
        if root.sum == 0 || root.sum >= 1 << 31 {
            return Err(invalid("MID state principal must be in 1..2^31 satoshis"));
        }
        if ctx.funds() != Amount::from_sat(root.sum) {
            return Err(invalid(
                "funding amount does not equal the LayerTree root sum",
            ));
        }
        let policy = ScriptPolicy::Or(
            self.config
                .transitions()
                .into_iter()
                .map(|transition| self.program(transition).map(ScriptPolicy::from))
                .collect::<Result<_, _>>()?,
        );
        StateContract {
            opening: NumsInternalKey::new(tree::state_tweak(root.hash)).map_err(invalid)?,
            principal: Amount::from_sat(root.sum),
            policy,
        }
        .compile(ctx)
    }
}

/// Obtain the physical oracle-leaf Merkle commitment used as Script witness `m`.
///
/// Validation binds this commitment to the descriptor, complete program
/// policies and public NUMS opening before any witness is constructed.
pub fn script_tree_root(compiled: &Compiled) -> Result<[u8; 32], CompilationError> {
    use bitcoin::hashes::Hash;
    compiled.validate()?;
    if compiled.nums_internal_key.is_none() {
        return Err(invalid("LayerTree output requires a public NUMS opening"));
    }
    let descriptor = compiled
        .descriptor
        .as_ref()
        .ok_or_else(|| invalid("LayerTree output requires its complete descriptor"))?;
    let mut input = bitcoin::psbt::Input::default();
    descriptor.update_psbt_input(&mut input).map_err(invalid)?;
    input
        .tap_merkle_root
        .map(|root| root.to_byte_array())
        .ok_or_else(|| invalid("LayerTree output requires a nonempty script tree"))
}

/// The physical output script independently derived by unchanged MID Script.
pub fn state_script(root: Node, scripts: [u8; 32]) -> Result<ScriptBuf, CompilationError> {
    tree::tree_spk(root.hash, scripts).map_err(invalid)
}

struct StateContract {
    opening: NumsInternalKey,
    principal: Amount,
    policy: ScriptPolicy,
}

impl StateContract {
    fn policy() -> Option<Guard<Self>> {
        Some(Guard::CachedPolicy(
            |contract| Ok(contract.policy.clone()),
            None,
        ))
    }
}

impl Contract for StateContract {
    sapio::declare! {finish, Self::policy}

    fn nums_internal_key(&self, _: &Context) -> Result<Option<NumsInternalKey>, CompilationError> {
        Ok(Some(self.opening))
    }

    fn ensure_amount(&self, _: Context) -> Result<Amount, CompilationError> {
        Ok(self.principal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{context, key};
    use bitcoin::bip32::Xpriv;
    use bitcoin::Network;
    use sapio_base::program::ProgramSpendPath;
    use sapio_base::secp::secp;

    fn terms(depth: u8) -> LayerTree {
        let oracle = Xpriv::new_master(Network::Regtest, &[81; 32]).unwrap();
        LayerTree::new(
            Config {
                depth,
                operator: key(82),
                tree_id: [83; 32],
                dust_min: 546,
            },
            Xpub::from_priv(secp(), &oracle),
        )
        .unwrap()
    }

    fn leaf(amount: u64, owner: u8) -> Leaf {
        Leaf {
            amount,
            exit_key: key(owner),
            owner_key: key(owner),
            evictor_key: key(82),
            evict_after: 200,
            nonce: 0,
            seq: 0,
        }
    }

    #[test]
    fn physical_outputs_equal_mid_tree_spk_and_preserve_the_script_set() {
        let terms = terms(2);
        let mut state = Tree::new(2).unwrap();
        state.set(0, Some(leaf(30_000, 84))).unwrap();
        state.set(1, Some(leaf(20_000, 85))).unwrap();
        let before = terms.compile_state(context(50_000), &state).unwrap();
        let scripts = script_tree_root(&before).unwrap();
        assert_eq!(
            ScriptBuf::from(&before.address),
            state_script(state.root(), scripts).unwrap()
        );
        assert_eq!(
            before.nums_internal_key.unwrap().internal_key().unwrap(),
            tree::state_internal_key(state.root().hash).unwrap()
        );
        assert_eq!(before.required_input_amount, Amount::from_sat(50_000));
        assert_eq!(
            before.program_policies.len(),
            terms.config().transitions().len()
        );
        assert!(before
            .program_requirements()
            .unwrap()
            .iter()
            .all(|requirement| matches!(requirement.path, ProgramSpendPath::ScriptPath(_))));

        state.set(0, Some(leaf(30_000, 86))).unwrap();
        let after = terms.compile_state(context(50_000), &state).unwrap();
        assert_ne!(before.address, after.address);
        assert_eq!(script_tree_root(&after).unwrap(), scripts);
        assert_eq!(before.program_policies, after.program_policies);
        assert_eq!(
            ScriptBuf::from(&after.address),
            state_script(state.root(), scripts).unwrap()
        );
        let decoded: Compiled =
            serde_json::from_value(serde_json::to_value(&after).unwrap()).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded, after);
    }

    #[test]
    fn state_compilation_rejects_inconsistent_funding_and_depth() {
        let terms = terms(2);
        let mut state = Tree::new(2).unwrap();
        assert!(terms.compile_state(context(0), &state).is_err());
        state.set(0, Some(leaf(10_000, 84))).unwrap();
        assert!(terms.compile_state(context(9_999), &state).is_err());
        assert!(terms.compile_state(context(10_001), &state).is_err());
        let mut different_depth = Tree::new(1).unwrap();
        different_depth.set(0, Some(leaf(10_000, 84))).unwrap();
        assert!(terms
            .compile_state(context(10_000), &different_depth)
            .is_err());
        assert!(terms.program(Transition::Exit { count: 3 }).is_err());
    }
}
