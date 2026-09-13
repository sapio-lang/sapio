#![allow(dead_code)]

#[path = "covenant.rs"]
mod covenant;

use bitcoin::blockdata::opcodes::all;
use bitcoin::blockdata::script::{Builder, Instruction};
use bitcoin::hashes::sha256;
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::{Keypair, Message, Secp256k1, SecretKey};
use bitcoin::sighash::{Prevouts, SighashCache};
use bitcoin::taproot::{LeafVersion, TapLeafHash};
use bitcoin::{Amount, Network, OutPoint, TapSighashType, Transaction, TxIn, TxOut, Witness};
use bitcoin::{ScriptBuf, XOnlyPublicKey};
use sapio::contract::abi::object::{RawTaproot, SupportedDescriptors};
use sapio::contract::{Compilable, Compiled, Context, Contract};
use sapio::{declare, guard, then};
use sapio_base::covenant::Ctv;
use sapio_base::covenant::LoweringPlan;
use sapio_base::policy::{PolicyCompiler, PolicyError, ScriptFragment, ScriptPolicy};
use sapio_base::Clause;
use sapio_ctv_emulator_trait::{CTVEmulator, EmulatorError};
use std::sync::Arc;

pub fn keypair(byte: u8) -> Keypair {
    Keypair::from_secret_key(
        &Secp256k1::new(),
        &SecretKey::from_slice(&[byte; 32]).unwrap(),
    )
}

pub fn key(byte: u8) -> XOnlyPublicKey {
    keypair(byte).x_only_public_key().0
}

/// A small external policy language: CHECKSIG's Boolean becomes 1 or 2,
/// and equality with 2 preserves exactly the original signature condition.
pub struct ArithmeticSigner(pub XOnlyPublicKey);

impl PolicyCompiler for ArithmeticSigner {
    fn compile_policy(&self) -> Result<ScriptPolicy, PolicyError> {
        ScriptFragment::new(
            Builder::new()
                .push_slice(self.0.serialize())
                .push_opcode(all::OP_CHECKSIG)
                .push_opcode(all::OP_1ADD)
                .push_int(2)
                .push_opcode(all::OP_NUMEQUAL)
                .into_script(),
        )
        .map(Into::into)
    }
}

pub struct Emulated;

impl CTVEmulator for Emulated {
    fn get_signer_for(&self, hash: sha256::Hash) -> Result<Clause, EmulatorError> {
        Ok(covenant::plan(4).lower_ctv(Ctv(hash)).unwrap())
    }

    fn sign(&self, psbt: Psbt) -> Result<Psbt, EmulatorError> {
        // Fixture signatures are added explicitly after binding. This mock
        // represents the emulator's clause, not its transaction approval logic.
        Ok(psbt)
    }
}

pub fn context(emulated: bool) -> Context {
    Context::new(
        Network::Regtest,
        Amount::from_sat(10_000),
        if emulated {
            covenant::plan(4)
        } else {
            LoweringPlan::Native
        },
        "custom".try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    )
}

pub struct Pure;

impl Pure {
    #[guard(policy)]
    fn owner(self, _ctx: Context) -> ArithmeticSigner {
        ArithmeticSigner(key(1))
    }
}

impl Contract for Pure {
    declare! {finish, Self::owner}
}

pub struct Protected {
    pub require_feerate: bool,
}

impl Protected {
    #[guard(policy)]
    fn owner(self, _ctx: Context) -> ArithmeticSigner {
        ArithmeticSigner(key(1))
    }

    #[guard]
    fn action_signer(self, _ctx: Context) {
        Clause::Key(key(2))
    }

    #[then(guarded_by = "[Self::owner, Self::action_signer]")]
    fn pay(self, ctx: Context) {
        let mut builder = ctx
            .template()
            .add_guard(ArithmeticSigner(key(3)).compile_policy()?)
            .add_output(Amount::from_sat(9_000), &key(9), None)?
            .add_fees(Amount::from_sat(1_000))?;
        if self.require_feerate {
            builder = builder.set_min_feerate(Amount::from_sat(1));
        }
        builder.into()
    }
}

impl Contract for Protected {
    declare! {actions, Self::pay}
}

pub fn compiled(protected: bool, emulated: bool) -> Compiled {
    let compiled = if protected {
        Protected {
            require_feerate: false,
        }
        .compile(context(emulated))
    } else {
        Pure.compile(context(emulated))
    }
    .unwrap();
    compiled.validate().unwrap();
    compiled
}

pub fn tree(compiled: &Compiled) -> &RawTaproot {
    match compiled.descriptor.as_ref().unwrap() {
        SupportedDescriptors::Taproot(tree) => tree,
        _ => panic!("custom policy must retain raw Taproot spending data"),
    }
}

pub fn funding(compiled: &Compiled) -> Transaction {
    Transaction {
        version: bitcoin::transaction::Version(2),
        lock_time: bitcoin::absolute::LockTime::from_consensus(0),
        input: vec![TxIn::default()],
        output: vec![TxOut {
            value: bitcoin::Amount::from_sat(10_000),
            script_pubkey: tree(compiled).script_pubkey(),
        }],
    }
}

pub fn unsigned_spend(compiled: &Compiled, outpoint: OutPoint) -> Transaction {
    let mut transaction = match compiled.ctv_to_tx.values().next() {
        Some(template) => template.tx.clone(),
        None => Transaction {
            version: bitcoin::transaction::Version(2),
            lock_time: bitcoin::absolute::LockTime::from_consensus(0),
            input: vec![TxIn::default()],
            output: vec![TxOut {
                value: bitcoin::Amount::from_sat(9_000),
                script_pubkey: ScriptBuf::new_p2tr(&Secp256k1::new(), key(9), None),
            }],
        },
    };
    transaction.input[0].previous_output = outpoint;
    transaction
}

pub fn script_signers(script: &ScriptBuf) -> Vec<XOnlyPublicKey> {
    let instructions: Vec<_> = script.instructions().collect::<Result<_, _>>().unwrap();
    instructions
        .windows(2)
        .filter_map(|pair| match pair {
            [Instruction::PushBytes(bytes), Instruction::Op(opcode)]
                if bytes.len() == 32
                    && [
                        all::OP_CHECKSIG,
                        all::OP_CHECKSIGVERIFY,
                        all::OP_CHECKSIGADD,
                    ]
                    .contains(opcode) =>
            {
                Some(XOnlyPublicKey::from_slice(bytes.as_bytes()).unwrap())
            }
            _ => None,
        })
        .collect()
}

pub fn covenant_signer(compiled: &Compiled) -> XOnlyPublicKey {
    covenant::key(4, *compiled.ctv_to_tx.keys().next().unwrap())
}

/// Construct the backend's witness explicitly; the Miniscript finalizer does
/// not support this arithmetic fragment. Bitcoin Core is the execution oracle.
pub fn signed_spend(
    compiled: &Compiled,
    mut transaction: Transaction,
    missing: Option<u8>,
    wrong_owner: bool,
) -> Transaction {
    let tree = tree(compiled);
    assert_eq!(tree.leaves().len(), 1);
    let script = &tree.leaves()[0].1;
    let prevouts = funding(compiled).output;
    let leaf = TapLeafHash::from_script(script, LeafVersion::TapScript);
    let hash = SighashCache::new(&transaction)
        .taproot_script_spend_signature_hash(
            0,
            &Prevouts::All(&prevouts),
            leaf,
            TapSighashType::Default,
        )
        .unwrap();
    let message = Message::from_digest_slice(&hash[..]).unwrap();
    let secp = Secp256k1::new();
    let mut witness: Vec<Vec<u8>> = script_signers(script)
        .into_iter()
        .rev()
        .map(|public_key| {
            let owner = (1..=3)
                .find(|owner| key(*owner) == public_key)
                .unwrap_or_else(|| {
                    assert_eq!(public_key, covenant_signer(compiled));
                    4
                });
            if missing == Some(owner) {
                vec![]
            } else {
                let signer = if owner == 4 {
                    covenant::keypair(4, *compiled.ctv_to_tx.keys().next().unwrap())
                } else {
                    keypair(if wrong_owner && owner == 1 { 8 } else { owner })
                };
                secp.sign_schnorr_no_aux_rand(&message, &signer)
                    .as_ref()
                    .to_vec()
            }
        })
        .collect();
    witness.push(script.as_bytes().to_vec());
    witness.push(
        tree.spend_info()
            .control_block(&(script.clone(), LeafVersion::TapScript))
            .unwrap()
            .serialize(),
    );
    transaction.input[0].witness = Witness::from_slice(&witness);
    transaction
}
