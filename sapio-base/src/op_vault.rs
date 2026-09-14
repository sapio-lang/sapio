//! Public constructors for a restricted historical BIP345 vault emulator.
//!
//! The WASM predicate implements a dynamic leaf update and recovery, rather
//! than precommitting a withdrawal destination. It accepts exactly one P2TR
//! vault input and native witness-v0 fee sponsors. Trigger and recovery retain
//! every satoshi of vault principal; final withdrawal uses the distributed CTV
//! emulator. Batching and Bitcoin relay policy are outside this profile.

use crate::program::{ctv_wasm_instance, EmulatedProgram, ProgramError, ProgramInstance};
use crate::Ctv;
use bitcoin::bip32::Xpub;
use bitcoin::consensus::Encodable;
use bitcoin::hashes::{sha256, Hash, HashEngine};
use bitcoin::key::TapTweak;
use bitcoin::opcodes::all::{OP_CHECKSIGVERIFY, OP_CSV};
use bitcoin::script::Builder;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::taproot::{ControlBlock, LeafVersion, TapNodeHash};
use bitcoin::{Amount, Script, ScriptBuf, XOnlyPublicKey};
use std::fmt;

/// Exact distributed WASM-v2 vault predicate, including its context imports.
pub const OP_VAULT_WASM: &[u8] = include_bytes!("../../evaluators/artifacts/op_vault.wasm");
const _: () = assert!(OP_VAULT_WASM.len() <= crate::program::MAX_PROGRAM_BYTES);

/// Invalid public vault terms or recovery evidence.
#[derive(Debug)]
pub enum VaultError {
    /// A relative block delay must be positive.
    ZeroDelay,
    /// An exact public program key could not be constructed.
    Program(ProgramError),
    /// This profile supports ordinary tapscript leaves up to 10,000 bytes.
    Leaf,
    /// The provided source leaf does not open the supplied spent P2TR output.
    SourceProof,
    /// Revaulting needs a positive amount at an output distinct from trigger.
    Revault,
    /// An output index is outside the positive ScriptNum domain.
    OutputIndex,
}

impl From<ProgramError> for VaultError {
    fn from(error: ProgramError) -> Self {
        Self::Program(error)
    }
}
impl fmt::Display for VaultError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDelay => formatter.write_str("vault delay must be at least one block"),
            Self::Program(error) => write!(formatter, "vault program: {error}"),
            Self::Leaf => {
                formatter.write_str("vault proof requires a tapscript leaf of at most 10000 bytes")
            }
            Self::SourceProof => {
                formatter.write_str("vault source leaf proof does not match the spent output")
            }
            Self::Revault => {
                formatter.write_str("revault needs a positive amount and a distinct output")
            }
            Self::OutputIndex => {
                formatter.write_str("vault output index exceeds the ScriptNum domain")
            }
        }
    }
}
impl std::error::Error for VaultError {}

/// Immutable delay and public oracle root for dynamic withdrawal triggers.
#[derive(Clone, Debug)]
pub struct Trigger {
    delay_blocks: u16,
    program: EmulatedProgram,
}

impl Trigger {
    /// Bind a positive delay and the oracle used for trigger and CTV evaluation.
    ///
    /// The parameter codec is `00 || delay:u16le || root:78-byte BIP32`.
    pub fn new(delay_blocks: u16, oracle_root: Xpub) -> Result<Self, VaultError> {
        if delay_blocks == 0 {
            return Err(VaultError::ZeroDelay);
        }
        let mut parameters = vec![0];
        parameters.extend_from_slice(&delay_blocks.to_le_bytes());
        parameters.extend_from_slice(&oracle_root.encode());
        let instance = ProgramInstance::wasm_v2(OP_VAULT_WASM.to_vec(), parameters)?;
        Ok(Self {
            delay_blocks,
            program: EmulatedProgram::new(instance, oracle_root)?,
        })
    }

    /// Fixed relative delay, measured in blocks after the pending output confirms.
    pub fn delay_blocks(&self) -> u16 {
        self.delay_blocks
    }

    /// Borrow the exact trigger predicate and its public oracle root.
    pub fn program(&self) -> &EmulatedProgram {
        &self.program
    }

    /// Borrow the trigger instance used by the signing request.
    pub fn instance(&self) -> &ProgramInstance {
        self.program.instance()
    }

    /// Predicate authorizing the final withdrawal template chosen at trigger.
    pub fn withdrawal_program(&self, expected: Ctv) -> Result<EmulatedProgram, VaultError> {
        Ok(EmulatedProgram::new(
            ctv_wasm_instance(expected),
            *self.program.root(),
        )?)
    }

    /// The exact delayed signature leaf installed by this evaluator.
    ///
    /// It is the Miniscript `and_v(v:pk(KEY),older(DELAY))`. KEY commits to
    /// the distributed inline-CTV evaluator and the selected BIP119 hash.
    pub fn withdrawal_script(&self, expected: Ctv) -> Result<ScriptBuf, VaultError> {
        let key = self.withdrawal_program(expected)?.derive_public_key()?;
        Ok(Builder::new()
            .push_x_only_key(&key)
            .push_opcode(OP_CHECKSIGVERIFY)
            .push_int(i64::from(self.delay_blocks))
            .push_opcode(OP_CSV)
            .into_script())
    }

    /// Encode trigger evidence with no script-witness or PSBT ambiguity.
    ///
    /// The codec is `hash:32 || trigger:u32le || revault:u32le || amount:u64le
    /// || source_len:u32le || source || control_len:u32le || control`.
    /// Absent revault is encoded as index `0xffffffff` and amount zero. The
    /// evaluator checks this evidence against its authenticated signing leaf.
    pub fn witness(
        &self,
        expected: Ctv,
        trigger_vout: u32,
        revault: Option<(u32, Amount)>,
        source_script: &Script,
        control: &ControlBlock,
    ) -> Result<Vec<u8>, VaultError> {
        check_leaf(source_script, control)?;
        check_index(trigger_vout)?;
        let (revault_vout, amount) = if let Some((index, amount)) = revault {
            check_index(index)?;
            if index == trigger_vout || amount == Amount::ZERO || amount > Amount::MAX_MONEY {
                return Err(VaultError::Revault);
            }
            (index, amount.to_sat())
        } else {
            (u32::MAX, 0)
        };
        let control = control.serialize();
        let mut bytes = Vec::with_capacity(56 + source_script.len() + control.len());
        bytes.extend_from_slice(expected.0.as_byte_array());
        bytes.extend_from_slice(&trigger_vout.to_le_bytes());
        bytes.extend_from_slice(&revault_vout.to_le_bytes());
        bytes.extend_from_slice(&amount.to_le_bytes());
        bytes.extend_from_slice(&(source_script.len() as u32).to_le_bytes());
        bytes.extend_from_slice(source_script.as_bytes());
        bytes.extend_from_slice(&(control.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&control);
        Ok(bytes)
    }

    /// Reconstruct the updated output from a verified original inclusion proof.
    ///
    /// The oracle separately binds the original leaf to the actual signing
    /// path. This offline helper can only verify the supplied inclusion proof.
    pub fn replacement_script_pubkey(
        &self,
        expected: Ctv,
        source_script: &Script,
        control: &ControlBlock,
        source_prevout_script: &Script,
    ) -> Result<ScriptBuf, VaultError> {
        check_leaf(source_script, control)?;
        if !source_prevout_script.is_p2tr() {
            return Err(VaultError::SourceProof);
        }
        let original_key = XOnlyPublicKey::from_slice(&source_prevout_script.as_bytes()[2..])
            .map_err(|_| VaultError::SourceProof)?;
        let secp = Secp256k1::verification_only();
        if !control.verify_taproot_commitment(&secp, original_key, source_script) {
            return Err(VaultError::SourceProof);
        }
        let replacement = self.withdrawal_script(expected)?;
        let mut root = TapNodeHash::from_script(&replacement, LeafVersion::TapScript);
        for sibling in &control.merkle_branch {
            root = TapNodeHash::from_node_hashes(root, *sibling);
        }
        let (key, _) = control.internal_key.tap_tweak(&secp, Some(root));
        Ok(ScriptBuf::new_p2tr_tweaked(key))
    }
}

/// A fixed recovery destination under an explicit public emulation oracle.
///
/// Initiating recovery may be gated by a separate signature policy. That
/// authorization key need not be a key controlling the destination wallet.
#[derive(Clone, Debug)]
pub struct Recovery {
    program: EmulatedProgram,
    commitment: sha256::Hash,
}

impl Recovery {
    /// Commit the destination's BIP345 recovery hash.
    /// The parameter codec is `01 || recovery_hash:32`.
    pub fn new(destination: &Script, oracle_root: Xpub) -> Result<Self, VaultError> {
        if destination.len() > 10_000 {
            return Err(VaultError::Leaf);
        }
        let commitment = recovery_hash(destination);
        let mut parameters = vec![1];
        parameters.extend_from_slice(commitment.as_byte_array());
        let instance = ProgramInstance::wasm_v2(OP_VAULT_WASM.to_vec(), parameters)?;
        Ok(Self {
            program: EmulatedProgram::new(instance, oracle_root)?,
            commitment,
        })
    }

    /// Borrow the exact recovery predicate and public oracle root.
    pub fn program(&self) -> &EmulatedProgram {
        &self.program
    }

    /// Borrow the recovery instance used by a signing request.
    pub fn instance(&self) -> &ProgramInstance {
        self.program.instance()
    }

    /// The tagged destination-script commitment.
    pub fn commitment(&self) -> sha256::Hash {
        self.commitment
    }

    /// Encode the chosen recovery output as exactly four little-endian bytes.
    pub fn witness(&self, output_index: u32) -> Result<Vec<u8>, VaultError> {
        check_index(output_index)?;
        Ok(output_index.to_le_bytes().to_vec())
    }
}

/// BIP345 `VaultRecoverySPK` tagged hash of CompactSize length plus script.
pub fn recovery_hash(script: &Script) -> sha256::Hash {
    let tag = sha256::Hash::hash(b"VaultRecoverySPK");
    let mut engine = sha256::Hash::engine();
    engine.input(tag.as_byte_array());
    engine.input(tag.as_byte_array());
    script
        .consensus_encode(&mut engine)
        .expect("hash engines accept every byte");
    sha256::Hash::from_engine(engine)
}

fn check_leaf(script: &Script, control: &ControlBlock) -> Result<(), VaultError> {
    if script.len() > 10_000 || control.leaf_version != LeafVersion::TapScript {
        return Err(VaultError::Leaf);
    }
    Ok(())
}
fn check_index(index: u32) -> Result<(), VaultError> {
    if index > i32::MAX as u32 {
        Err(VaultError::OutputIndex)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{miniscript, Clause};
    use bitcoin::bip32::Xpriv;
    use bitcoin::taproot::TaprootBuilder;
    use bitcoin::Network;

    fn root() -> Xpub {
        Xpub::from_priv(
            &Secp256k1::new(),
            &Xpriv::new_master(Network::Regtest, &[42; 32]).unwrap(),
        )
    }

    #[test]
    fn public_terms_validate_delay_and_derivation_depth() {
        assert!(matches!(
            Trigger::new(0, root()),
            Err(VaultError::ZeroDelay)
        ));
        assert!(Trigger::new(65535, root()).is_ok());
        let mut deep = root();
        deep.depth = crate::program::MAX_PROGRAM_ROOT_DEPTH + 1;
        assert!(Trigger::new(10, deep).is_err());
        assert!(Recovery::new(Script::new(), deep).is_err());
    }

    #[test]
    fn delayed_leaf_matches_the_miniscript_compiler_and_commits_the_ctv_hash() {
        let expected = Ctv(sha256::Hash::hash(b"withdrawal"));
        for delay in [1, 16, 17, 127, 128, 32767, 32768, 65535] {
            let trigger = Trigger::new(delay, root()).unwrap();
            let key = trigger
                .withdrawal_program(expected)
                .unwrap()
                .derive_public_key()
                .unwrap();
            let policy = Clause::And(vec![
                Clause::Key(key).into(),
                Clause::Older(miniscript::RelLockTime::from_height(delay)).into(),
            ]);
            assert_eq!(
                trigger.withdrawal_script(expected).unwrap(),
                policy.compile::<miniscript::Tap>().unwrap().encode()
            );
            assert_ne!(
                trigger.withdrawal_script(expected).unwrap(),
                trigger
                    .withdrawal_script(Ctv(sha256::Hash::hash(b"different")))
                    .unwrap()
            );
        }
    }

    #[test]
    fn leaf_replacement_keeps_the_original_recovery_sibling() {
        let trigger = Trigger::new(144, root()).unwrap();
        let expected = Ctv(sha256::Hash::hash(b"withdrawal"));
        let source = Builder::new()
            .push_x_only_key(&trigger.program().derive_public_key().unwrap())
            .push_opcode(bitcoin::opcodes::all::OP_CHECKSIG)
            .into_script();
        let recovery = Builder::new().push_int(1).into_script();
        let key = root().to_x_only_pub();
        let secp = Secp256k1::verification_only();
        let tree = TaprootBuilder::new()
            .add_leaf(1, source.clone())
            .unwrap()
            .add_leaf(1, recovery.clone())
            .unwrap()
            .finalize(&secp, key)
            .unwrap();
        let source_output = ScriptBuf::new_p2tr_tweaked(tree.output_key());
        let control = tree
            .control_block(&(source.clone(), LeafVersion::TapScript))
            .unwrap();
        let expected_tree = TaprootBuilder::new()
            .add_leaf(1, trigger.withdrawal_script(expected).unwrap())
            .unwrap()
            .add_leaf(1, recovery)
            .unwrap()
            .finalize(&secp, key)
            .unwrap();
        assert_eq!(
            trigger
                .replacement_script_pubkey(expected, &source, &control, &source_output)
                .unwrap(),
            ScriptBuf::new_p2tr_tweaked(expected_tree.output_key())
        );
        let witness = trigger
            .witness(
                expected,
                0,
                Some((1, Amount::from_sat(1000))),
                &source,
                &control,
            )
            .unwrap();
        assert_eq!(
            &witness[32..48],
            &[
                &0_u32.to_le_bytes()[..],
                &1_u32.to_le_bytes(),
                &1000_u64.to_le_bytes()
            ]
            .concat()
        );
        assert!(trigger
            .witness(
                expected,
                0,
                Some((0, Amount::from_sat(1))),
                &source,
                &control
            )
            .is_err());
        assert!(trigger
            .witness(expected, 0, Some((1, Amount::ZERO)), &source, &control)
            .is_err());
    }

    #[test]
    fn recovery_commitment_includes_compact_size_and_exact_script() {
        for length in [0, 1, 252, 253, 10000] {
            let script = ScriptBuf::from_bytes(vec![0x51; length]);
            let recovery = Recovery::new(&script, root()).unwrap();
            let tag = sha256::Hash::hash(b"VaultRecoverySPK");
            let mut encoded = Vec::new();
            encoded.extend_from_slice(tag.as_byte_array());
            encoded.extend_from_slice(tag.as_byte_array());
            script.consensus_encode(&mut encoded).unwrap();
            assert_eq!(recovery.commitment(), sha256::Hash::hash(&encoded));
            assert_eq!(recovery.witness(2).unwrap(), [2, 0, 0, 0]);
        }
    }
}
