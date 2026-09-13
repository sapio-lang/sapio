use bitcoin::key::TapTweak;
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::{Keypair, Message, Secp256k1, SecretKey};
use bitcoin::sighash::{Prevouts, SighashCache};
use bitcoin::taproot::TapLeafHash;
use bitcoin::{Amount, Network, OutPoint, TapSighashType};
use bitcoin::{ScriptBuf, Transaction, TxIn, TxOut, XOnlyPublicKey};
use sapio::contract::abi::object::{Object, SupportedDescriptors};
use sapio::contract::actions::Guard;
use sapio::contract::{Compilable, Context, DynamicContract};
use sapio_base::covenant::LoweringPlan;
use sapio_base::miniscript::descriptor::Tr;
use sapio_base::miniscript::ord::{envelope::Envelope, Inscription};
use sapio_base::miniscript::psbt::{interpreter_check, PsbtExt};
use sapio_base::miniscript::Descriptor;
use sapio_base::util::CTVHash;
use sapio_base::Clause;
use std::collections::BTreeSet;
use std::sync::Arc;

fn keypair(byte: u8) -> Keypair {
    Keypair::from_secret_key(
        &Secp256k1::new(),
        &SecretKey::from_slice(&[byte; 32]).unwrap(),
    )
}

fn key(byte: u8) -> XOnlyPublicKey {
    keypair(byte).x_only_public_key().0
}

fn guard_at<const SLOT: usize>() -> Option<Guard<[Clause; 3]>> {
    Some(Guard::Cache(|clauses| clauses[SLOT].clone(), None))
}

fn compile(clauses: [Clause; 3], order: &[usize]) -> Object {
    let factories = [guard_at::<0>, guard_at::<1>, guard_at::<2>];
    let contract = DynamicContract::<_> {
        actions: vec![],
        finish: order.iter().map(|index| factories[*index]).collect(),
        metadata_f: Box::new(|_, _| Ok(Default::default())),
        ensure_amount_f: Box::new(|_, ctx| Ok(ctx.funds())),
        data: clauses,
    };
    let compiled = contract
        .compile(Context::new(
            Network::Regtest,
            Amount::from_sat(10_000),
            LoweringPlan::Native,
            "lowering".try_into().unwrap(),
            Arc::new(Default::default()),
            None,
        ))
        .unwrap();
    compiled.validate().unwrap();
    compiled
}

fn tree(compiled: &Object) -> &Tr<XOnlyPublicKey> {
    match compiled.descriptor.as_ref().unwrap() {
        SupportedDescriptors::XOnly(Descriptor::Tr(tree)) => tree,
        _ => panic!("expected a Taproot descriptor"),
    }
}

fn assert_same_output(first: &Object, second: &Object) {
    assert_eq!(first.address, second.address);
    assert_eq!(first.descriptor, second.descriptor);
}

#[test]
fn reversing_finish_key_alternatives_preserves_the_output_and_internal_key() {
    let clauses = [
        Clause::Key(key(1)),
        Clause::Key(key(2)),
        Clause::Unsatisfiable,
    ];
    let forward = compile(clauses.clone(), &[0, 1]);
    let reversed = compile(clauses, &[1, 0]);
    assert_same_output(&forward, &reversed);
    assert_eq!(*tree(&forward).internal_key(), key(1).min(key(2)));
    assert_eq!(tree(&forward).leaves().count(), 2);
}

#[test]
fn repeated_script_alternatives_do_not_change_the_tree_or_output() {
    let clauses = [
        Clause::Key(key(1)),
        Clause::And(vec![
            Arc::new(Clause::Key(key(2))),
            Arc::new(Clause::Older(
                sapio::miniscript::RelLockTime::from_consensus(16).unwrap(),
            )),
        ]),
        Clause::Key(key(3)),
    ];
    let canonical = compile(clauses.clone(), &[0, 1, 2]);
    for order in [&[2, 0, 1][..], &[1, 2, 0, 1, 0, 2][..]] {
        let compiled = compile(clauses.clone(), order);
        assert_same_output(&canonical, &compiled);
        assert_eq!(tree(&compiled).leaves().count(), 3);
    }
}

fn inscribe(body: &[u8], sub: Clause) -> Clause {
    Clause::Inscribe(
        Box::new(Inscription::new(None, Some(body.to_vec()))),
        Arc::new(sub),
    )
}

fn transaction() -> Transaction {
    Transaction {
        version: bitcoin::transaction::Version(2),
        lock_time: bitcoin::absolute::LockTime::from_consensus(0),
        input: vec![TxIn {
            sequence: bitcoin::Sequence(16),
            ..TxIn::default()
        }],
        output: vec![TxOut {
            value: bitcoin::Amount::from_sat(9_000),
            script_pubkey: ScriptBuf::new(),
        }],
    }
}

fn psbt(compiled: &Object, mut transaction: Transaction) -> Psbt {
    let tree = tree(compiled);
    let info = tree.spend_info();
    let funding = Transaction {
        version: bitcoin::transaction::Version(2),
        lock_time: bitcoin::absolute::LockTime::from_consensus(0),
        input: vec![TxIn::default()],
        output: vec![TxOut {
            value: bitcoin::Amount::from_sat(10_000),
            script_pubkey: tree.script_pubkey(),
        }],
    };
    transaction.input[0].previous_output = OutPoint::new(funding.compute_txid(), 0);
    let mut psbt = Psbt::from_unsigned_tx(transaction).unwrap();
    psbt.inputs[0].witness_utxo = Some(funding.output[0].clone());
    psbt.inputs[0].tap_internal_key = Some(*tree.internal_key());
    psbt.inputs[0].tap_merkle_root = info.merkle_root();
    for leaf in info.leaves() {
        let script = (leaf.script().to_owned(), leaf.leaf_version());
        psbt.inputs[0]
            .tap_scripts
            .insert(leaf.into_control_block(), script);
    }
    psbt
}

// Sign only after mutations, so negative timelock/CTV cases contain fresh,
// otherwise valid transaction signatures rather than merely stale signatures.
fn sign_scripts(psbt: &mut Psbt, owner: u8) {
    let secp = Secp256k1::new();
    let prevouts = [psbt.inputs[0].witness_utxo.clone().unwrap()];
    let leaves: Vec<_> = psbt.inputs[0]
        .tap_scripts
        .values()
        .map(|(script, version)| TapLeafHash::from_script(script, *version))
        .collect();
    for leaf in leaves {
        let hash_ty = TapSighashType::Default;
        let hash = SighashCache::new(&psbt.unsigned_tx)
            .taproot_script_spend_signature_hash(0, &Prevouts::All(&prevouts), leaf, hash_ty)
            .unwrap();
        let sig = secp.sign_schnorr_no_aux_rand(
            &Message::from_digest_slice(&hash[..]).unwrap(),
            &keypair(owner),
        );
        psbt.inputs[0].tap_script_sigs.insert(
            (key(owner), leaf),
            bitcoin::taproot::Signature {
                signature: sig,
                sighash_type: hash_ty,
            },
        );
    }
}

fn finalize(mut psbt: Psbt) -> Transaction {
    let secp = Secp256k1::new();
    psbt.finalize_mut(&secp).unwrap();
    interpreter_check(&psbt, &secp).unwrap();
    psbt.extract(&secp).unwrap()
}

fn sign_key_path(psbt: &mut Psbt, owner: u8) {
    let secp = Secp256k1::new();
    let prevouts = [psbt.inputs[0].witness_utxo.clone().unwrap()];
    let hash_ty = TapSighashType::Default;
    let hash = SighashCache::new(&psbt.unsigned_tx)
        .taproot_key_spend_signature_hash(0, &Prevouts::All(&prevouts), hash_ty)
        .unwrap();
    let tweaked_owner = keypair(owner)
        .tap_tweak(&secp, psbt.inputs[0].tap_merkle_root)
        .to_keypair();
    psbt.inputs[0].tap_key_sig = Some(bitcoin::taproot::Signature {
        signature: secp.sign_schnorr_no_aux_rand(
            &Message::from_digest_slice(&hash[..]).unwrap(),
            &tweaked_owner,
        ),
        sighash_type: hash_ty,
    });
}

#[test]
fn selected_bare_key_can_authorize_a_real_key_path_spend() {
    let compiled = compile(
        [
            Clause::Key(key(1)),
            Clause::Key(key(2)),
            Clause::Unsatisfiable,
        ],
        &[1, 0],
    );
    let owner = if key(1) < key(2) { 1 } else { 2 };
    let mut spend = psbt(&compiled, transaction());
    spend.inputs[0].tap_scripts.clear();
    sign_key_path(&mut spend, owner);
    assert_eq!(finalize(spend).input[0].witness.len(), 1);
}

#[test]
fn deduplicating_alternatives_preserves_inscription_order_and_multiplicity() {
    let clauses = [
        inscribe(b"a", inscribe(b"a", Clause::Key(key(1)))),
        inscribe(b"a", inscribe(b"b", Clause::Key(key(1)))),
        inscribe(b"b", inscribe(b"a", Clause::Key(key(1)))),
    ];
    let distinct = compile(clauses.clone(), &[0, 1, 2]);
    let repeated = compile(clauses, &[2, 0, 1, 0, 2, 1]);
    assert_same_output(&distinct, &repeated);
    assert_eq!(tree(&repeated).leaves().count(), 3);
    let mut revealed = BTreeSet::new();
    for leaf in tree(&repeated).leaves() {
        let miniscript = leaf.miniscript();
        let script = miniscript.encode();
        let mut spend = psbt(&repeated, transaction());
        spend.inputs[0]
            .tap_scripts
            .retain(|_, (candidate, _)| *candidate == script);
        sign_scripts(&mut spend, 1);
        let transaction = finalize(spend);
        assert_eq!(transaction.input[0].witness.to_vec()[1], script.as_bytes());
        let bodies: Vec<_> = Envelope::<Inscription>::from_transaction(&transaction)
            .into_iter()
            .map(|envelope| envelope.payload.body.unwrap())
            .collect();
        revealed.insert(bodies);
    }
    assert_eq!(
        revealed,
        BTreeSet::from([
            vec![b"a".to_vec(), b"a".to_vec()],
            vec![b"a".to_vec(), b"b".to_vec()],
            vec![b"b".to_vec(), b"a".to_vec()],
        ])
    );
}

#[derive(Clone, Copy, Debug)]
enum Constraint {
    RelativeLock,
    Covenant,
    Inscription,
}

#[test]
fn constrained_signers_cannot_bypass_the_script_through_key_path_spending() {
    let secp = Secp256k1::new();
    for constraint in [
        Constraint::RelativeLock,
        Constraint::Covenant,
        Constraint::Inscription,
    ] {
        let transaction = transaction();
        let policy = match constraint {
            Constraint::RelativeLock => Clause::And(vec![
                Arc::new(Clause::Key(key(1))),
                Arc::new(Clause::Older(
                    sapio::miniscript::RelLockTime::from_consensus(16).unwrap(),
                )),
            ]),
            Constraint::Covenant => Clause::And(vec![
                Arc::new(Clause::Key(key(1))),
                Arc::new(Clause::TxTemplate(transaction.get_ctv_hash(0))),
            ]),
            Constraint::Inscription => inscribe(b"reveal", Clause::Key(key(1))),
        };
        let compiled = compile([policy, Clause::Unsatisfiable, Clause::Unsatisfiable], &[0]);
        let tree = tree(&compiled);
        // This is the original SHA256([1; 32]) fallback point, not any owner's
        // key. Pinning it also prevents changing existing script-only outputs.
        assert_eq!(
            tree.internal_key().to_string(),
            "72cd6e8422c407fb6d098690f1130b7ded7ec2f7f5e1d30bd9d521f015363793"
        );
        assert_ne!(*tree.internal_key(), key(1));
        let unsigned = psbt(&compiled, transaction);
        assert!(unsigned.clone().finalize_mut(&secp).is_err());

        let mut script_spend = unsigned.clone();
        sign_scripts(&mut script_spend, 1);
        let valid = finalize(script_spend);
        assert_eq!(valid.input[0].witness.len(), 3, "{constraint:?}");

        let mut key_spend = unsigned.clone();
        key_spend.inputs[0].tap_scripts.clear();
        sign_key_path(&mut key_spend, 1);
        assert!(key_spend.finalize_mut(&secp).is_err(), "{constraint:?}");

        let mut invalid = unsigned;
        match constraint {
            Constraint::RelativeLock => {
                invalid.unsigned_tx.input[0].sequence = bitcoin::Sequence(15)
            }
            Constraint::Covenant => invalid.unsigned_tx.output[0].value -= bitcoin::Amount::ONE_SAT,
            Constraint::Inscription => {
                assert_eq!(
                    Envelope::<Inscription>::from_transaction(&valid)[0]
                        .payload
                        .body,
                    Some(b"reveal".to_vec())
                );
                invalid.inputs[0].tap_scripts.clear();
            }
        }
        sign_scripts(&mut invalid, 1);
        assert!(invalid.finalize_mut(&secp).is_err(), "{constraint:?}");
    }
}
