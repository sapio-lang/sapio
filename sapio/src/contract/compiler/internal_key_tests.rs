use super::*;
use crate::contract::actions::Guard;
use crate::contract::object::{Object, SupportedDescriptors};
use crate::contract::Contract;
use bitcoin::blockdata::opcodes::all::{OP_CHECKSIG, OP_DROP};
use bitcoin::blockdata::script::Builder;
use bitcoin::hashes::{sha256, Hash};
use bitcoin::psbt::PartiallySignedTransaction as Psbt;
use bitcoin::secp256k1::{Keypair, Message, Secp256k1, SecretKey};
use bitcoin::util::bip32::{ExtendedPrivKey, ExtendedPubKey};
use bitcoin::util::schnorr::TapTweak;
use bitcoin::util::sighash::{Prevouts, SighashCache};
use bitcoin::{
    Amount, Network, OutPoint, SchnorrSig, SchnorrSighashType, Script, Transaction, TxIn, TxOut,
};
use sapio_base::miniscript::ord::Inscription;
use sapio_base::miniscript::psbt::{interpreter_check, PsbtExt};
use sapio_base::policy::ScriptFragment;
use sapio_base::LoweringPlan;

struct PinnedContract {
    pin: Option<XOnlyPublicKey>,
    policies: [ScriptPolicy; 2],
}

impl PinnedContract {
    fn first() -> Option<Guard<Self>> {
        Some(Guard::CachedPolicy(
            |contract| Ok(contract.policies[0].clone()),
            None,
        ))
    }

    fn second() -> Option<Guard<Self>> {
        Some(Guard::CachedPolicy(
            |contract| Ok(contract.policies[1].clone()),
            None,
        ))
    }
}

impl Contract for PinnedContract {
    crate::declare! {non updatable}
    crate::declare! {finish, Self::first, Self::second}

    fn pinned_internal_key(
        &self,
        _ctx: &Context,
    ) -> Result<Option<XOnlyPublicKey>, CompilationError> {
        Ok(self.pin)
    }
}

fn keypair(byte: u8) -> Keypair {
    Keypair::from_secret_key(
        &Secp256k1::new(),
        &SecretKey::from_slice(&[byte; 32]).unwrap(),
    )
}

fn key(byte: u8) -> XOnlyPublicKey {
    keypair(byte).x_only_public_key().0
}

fn compile(
    pin: Option<XOnlyPublicKey>,
    policies: [ScriptPolicy; 2],
    lowering: LoweringPlan,
) -> Result<Object, CompilationError> {
    PinnedContract { pin, policies }.compile(Context::new(
        Network::Regtest,
        Amount::from_sat(10_000),
        lowering,
        "pinned_key".try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    ))
}

fn tree(object: &Object) -> &descriptor::Tr<XOnlyPublicKey> {
    match object.descriptor.as_ref().unwrap() {
        SupportedDescriptors::XOnly(Descriptor::Tr(tree)) => tree,
        _ => panic!("expected a Miniscript Taproot tree"),
    }
}

fn raw_key(owner: XOnlyPublicKey) -> ScriptPolicy {
    ScriptFragment::new(
        Builder::new()
            .push_int(1)
            .push_opcode(OP_DROP)
            .push_slice(&owner.serialize())
            .push_opcode(OP_CHECKSIG)
            .into_script(),
    )
    .unwrap()
    .into()
}

#[test]
fn pin_overrides_competing_bare_keys_and_removes_only_its_redundant_leaf() {
    let owner = key(1).max(key(2));
    let other = key(1).min(key(2));
    let policies = [Clause::Key(owner).into(), Clause::Key(other).into()];
    let automatic = compile(None, policies.clone(), LoweringPlan::Native).unwrap();
    assert_eq!(*tree(&automatic).internal_key(), other);
    assert_eq!(tree(&automatic).iter_scripts().count(), 2);
    let explicit = compile(Some(owner), policies.clone(), LoweringPlan::Native).unwrap();
    assert_eq!(*tree(&explicit).internal_key(), owner);
    let scripts: Vec<_> = tree(&explicit)
        .iter_scripts()
        .map(|(_, script)| script.encode())
        .collect();
    assert_eq!(
        scripts,
        [Clause::Key(other).compile::<Tap>().unwrap().encode()]
    );
    assert_ne!(explicit.address, automatic.address);
    let reversed = compile(
        Some(owner),
        [policies[1].clone(), policies[0].clone()],
        LoweringPlan::Native,
    )
    .unwrap();
    assert_eq!(explicit.address, reversed.address);
    assert_eq!(explicit.descriptor, reversed.descriptor);
    explicit.validate().unwrap();
    let decoded: Object = serde_json::from_value(serde_json::to_value(&explicit).unwrap()).unwrap();
    decoded.validate().unwrap();
    assert_eq!(explicit, decoded);
}

#[test]
fn pinned_key_only_output_has_no_redundant_tree_and_authorizes_a_real_spend() {
    let object = compile(
        Some(key(1)),
        [Clause::Key(key(1)).into(), Clause::Key(key(1)).into()],
        LoweringPlan::Native,
    )
    .unwrap();
    let tree = tree(&object);
    assert_eq!(*tree.internal_key(), key(1));
    assert_eq!(tree.iter_scripts().count(), 0);
    assert_eq!(tree.spend_info().merkle_root(), None);
    object.validate().unwrap();
    let previous = TxOut {
        value: 10_000,
        script_pubkey: tree.script_pubkey(),
    };
    let transaction = Transaction {
        version: 2,
        lock_time: 0,
        input: vec![TxIn {
            previous_output: OutPoint::default(),
            ..TxIn::default()
        }],
        output: vec![TxOut {
            value: 9_000,
            script_pubkey: Script::new(),
        }],
    };
    let mut psbt = Psbt::from_unsigned_tx(transaction).unwrap();
    psbt.inputs[0].witness_utxo = Some(previous.clone());
    psbt.inputs[0].tap_internal_key = Some(key(1));
    let hash_ty = SchnorrSighashType::All;
    let hash = SighashCache::new(&psbt.unsigned_tx)
        .taproot_key_spend_signature_hash(0, &Prevouts::All(&[previous]), hash_ty)
        .unwrap();
    let secp = Secp256k1::new();
    let tweaked = keypair(1).tap_tweak(&secp, None).into_inner();
    psbt.inputs[0].tap_key_sig = Some(SchnorrSig {
        sig: secp
            .sign_schnorr_no_aux_rand(&Message::from_digest_slice(&hash[..]).unwrap(), &tweaked),
        hash_ty,
    });
    psbt.finalize_mut(&secp).unwrap();
    interpreter_check(&psbt, &secp).unwrap();
    assert_eq!(psbt.extract(&secp).unwrap().input[0].witness.len(), 1);
}

#[test]
fn missing_constrained_and_opaque_keys_cannot_gain_key_path_authority() {
    let hash = sha256::Hash::hash(b"hashlock");
    let policies = [
        Clause::Key(key(2)).into(),
        Clause::And(vec![Clause::Key(key(1)), Clause::Older(16)]).into(),
        Clause::And(vec![Clause::Key(key(1)), Clause::Sha256(hash)]).into(),
        Clause::Threshold(2, vec![Clause::Key(key(1)), Clause::Key(key(2))]).into(),
        Clause::Inscribe(
            Box::new(Inscription::new(None, Some(b"reveal".to_vec()))),
            Box::new(Clause::Key(key(1))),
        )
        .into(),
        raw_key(key(1)),
    ];
    for policy in policies {
        let error = compile(
            Some(key(1)),
            [policy, Clause::Key(key(2)).into()],
            LoweringPlan::Native,
        )
        .unwrap_err();
        assert!(
            matches!(error, CompilationError::UnauthorizedInternalKey { key: requested } if requested == key(1)),
            "{error}"
        );
    }
}

#[test]
fn mixed_raw_tree_honors_pin_and_preserves_raw_and_constrained_leaves() {
    let raw = raw_key(key(2));
    let ScriptPolicy::Script(fragment) = &raw else {
        unreachable!()
    };
    let expected = fragment.as_script().clone();
    let compiled = compile(
        Some(key(1)),
        [Clause::Key(key(1)).into(), raw],
        LoweringPlan::Native,
    )
    .unwrap();
    let Some(SupportedDescriptors::Taproot(raw_tree)) = &compiled.descriptor else {
        panic!("expected raw tree")
    };
    assert_eq!(raw_tree.internal_key(), key(1));
    assert_eq!(raw_tree.leaves(), &[(0, expected)]);
    compiled.validate().unwrap();

    let constrained = Clause::And(vec![Clause::Key(key(1)), Clause::Older(16)]);
    let expected = constrained.compile::<Tap>().unwrap().encode();
    let compiled = compile(
        Some(key(1)),
        [Clause::Key(key(1)).into(), constrained.into()],
        LoweringPlan::Native,
    )
    .unwrap();
    assert_eq!(*tree(&compiled).internal_key(), key(1));
    assert_eq!(
        tree(&compiled).iter_scripts().next().unwrap().1.encode(),
        expected
    );
}

#[test]
fn explicit_owner_pin_survives_native_and_emulated_covenant_lowering() {
    let secp = Secp256k1::new();
    let root = ExtendedPubKey::from_priv(
        &secp,
        &ExtendedPrivKey::new_master(Network::Regtest, &[9; 32]).unwrap(),
    );
    let predicate = Ctv(sha256::Hash::hash(b"a template"));
    for lowering in [
        LoweringPlan::Native,
        LoweringPlan::CtvEmulation {
            signers: vec![root],
            threshold: 1,
        },
    ] {
        let native = lowering == LoweringPlan::Native;
        let compiled = compile(
            Some(key(1)),
            [
                Clause::Key(key(1)).into(),
                ScriptPolicy::from(Emulatable(predicate)),
            ],
            lowering,
        )
        .unwrap();
        assert_eq!(*tree(&compiled).internal_key(), key(1));
        assert_eq!(tree(&compiled).iter_scripts().count(), 1);
        assert_eq!(compiled.requires_native_ctv(), native);
        assert!(compiled
            .covenant_requirements
            .predicates
            .contains(&predicate));
        compiled.validate().unwrap();
    }
}
