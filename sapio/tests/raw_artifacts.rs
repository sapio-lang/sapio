use bitcoin::blockdata::opcodes::all::{OP_DROP, OP_NOP4};
use bitcoin::blockdata::script::Builder;
use bitcoin::hashes::sha256;
use bitcoin::hashes::Hash;
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
use bitcoin::taproot::LeafVersion;
use bitcoin::{Address, Amount, Network, OutPoint, ScriptBuf, Transaction, TxIn, TxOut, Txid};
use sapio::contract::abi::object::{
    ArtifactErrorKind, GuardMetadata, ObjectError, RawTaproot, SupportedDescriptors,
};
use sapio::contract::abi::studio::SapioStudioFormat;
use sapio::contract::{Compiled, Context};
use sapio::template::Template;
use sapio_base::covenant::{Ctv, LoweringPlan};
use sapio_base::policy::{ScriptFragment, ScriptPolicy};
use sapio_base::txindex::{TxIndex, TxIndexError};
use sapio_base::{CTVHash, Clause};
use sapio_ctv_emulator_trait::{CTVEmulator, EmulatorError};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn artifact(key_only: bool) -> Compiled {
    let secp = Secp256k1::new();
    let key = Keypair::from_secret_key(&secp, &SecretKey::from_slice(&[1; 32]).unwrap())
        .x_only_public_key()
        .0;
    let destination = Compiled::from_address(
        Address::p2tr(&secp, key, None, Network::Regtest),
        bitcoin::Amount::ZERO,
    );
    let template: Template = Context::new(
        Network::Regtest,
        Amount::from_sat(1_000),
        LoweringPlan::Native,
        "raw".try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    )
    .template()
    .add_output(Amount::from_sat(1_000), &destination, None)
    .unwrap()
    .into();
    let script = Builder::new()
        .push_slice(template.hash().to_byte_array())
        .push_opcode(OP_NOP4);
    let ctv = script.clone().into_script();
    let alternate = script.push_opcode(OP_DROP).push_int(1).into_script();
    let leaves = if key_only {
        vec![]
    } else {
        // A repeated script at unequal depths must retain both control blocks.
        vec![(1, ctv.clone()), (2, alternate), (2, ctv)]
    };
    let raw = RawTaproot::new(key, leaves).unwrap();
    let mut object = Compiled::from_address(
        Address::from_script(&raw.script_pubkey(), Network::Regtest).unwrap(),
        template.required_input_amount,
    );
    object.descriptor = Some(raw.into());
    object
        .covenant_requirements
        .predicates
        .insert(Ctv(template.hash()));
    object.ctv_to_tx.insert(template.hash(), template);
    object
}

fn funding(object: &Compiled) -> Transaction {
    Transaction {
        version: bitcoin::transaction::Version(2),
        lock_time: bitcoin::absolute::LockTime::from_consensus(0),
        input: vec![TxIn::default()],
        output: vec![TxOut {
            value: bitcoin::Amount::from_sat(1_000),
            script_pubkey: ScriptBuf::from(&object.address),
        }],
    }
}

struct Index {
    transactions: RefCell<BTreeMap<Txid, Arc<Transaction>>>,
    lookups: Cell<usize>,
    writes: Cell<usize>,
}

impl Index {
    fn with(tx: Transaction) -> Rc<Self> {
        Rc::new(Self {
            transactions: RefCell::new(BTreeMap::from([(tx.compute_txid(), Arc::new(tx))])),
            lookups: Cell::new(0),
            writes: Cell::new(0),
        })
    }
}

impl TxIndex for Index {
    fn lookup_tx(&self, txid: &Txid) -> Result<Arc<Transaction>, TxIndexError> {
        self.lookups.set(self.lookups.get() + 1);
        self.transactions
            .borrow()
            .get(txid)
            .cloned()
            .ok_or(TxIndexError::UnknownTxid(*txid))
    }

    fn add_tx(&self, tx: Arc<Transaction>) -> Result<Txid, TxIndexError> {
        self.writes.set(self.writes.get() + 1);
        let txid = tx.compute_txid();
        self.transactions.borrow_mut().insert(txid, tx);
        Ok(txid)
    }
}

#[derive(Default)]
struct Signer {
    calls: AtomicUsize,
    strip_scripts: bool,
}

impl CTVEmulator for Signer {
    fn get_signer_for(&self, hash: sha256::Hash) -> Result<Clause, EmulatorError> {
        Ok(Clause::TxTemplate(hash))
    }

    fn sign(&self, mut psbt: Psbt) -> Result<Psbt, EmulatorError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.strip_scripts {
            psbt.inputs[0].tap_scripts.clear();
        }
        Ok(psbt)
    }
}

#[test]
fn serialized_raw_artifacts_bind_all_control_blocks_and_authenticated_funding() {
    for key_only in [false, true] {
        let object: Compiled =
            serde_json::from_slice(&serde_json::to_vec(&artifact(key_only)).unwrap()).unwrap();
        object.validate().unwrap();
        let Some(SupportedDescriptors::Taproot(raw)) = &object.descriptor else {
            panic!("round trip lost raw Taproot spending data");
        };
        let previous = funding(&object);
        let outpoint = OutPoint::new(previous.compute_txid(), 0);
        let index = Index::with(previous.clone());
        let signer = Signer::default();
        let program = object
            .bind_psbt(outpoint, BTreeMap::new(), index.clone(), &signer)
            .unwrap();
        let SapioStudioFormat::LinkedPSBT { psbt, .. } =
            &program.program.get(&object.root_path).unwrap().txs[0];
        let psbt: Psbt = Psbt::deserialize(&base64::decode(psbt).unwrap()).unwrap();
        let input = &psbt.inputs[0];
        assert_eq!(input.tap_internal_key, Some(raw.internal_key()));
        assert_eq!(input.tap_merkle_root, raw.spend_info().merkle_root());
        assert_eq!(input.tap_scripts.len(), if key_only { 0 } else { 3 });
        assert_eq!(input.witness_utxo.as_ref(), Some(&previous.output[0]));
        assert_eq!(input.non_witness_utxo.as_ref(), Some(&previous));
        assert_eq!(psbt.unsigned_tx.input[0].previous_output, outpoint);
        assert_eq!(
            psbt.unsigned_tx.get_ctv_hash(0),
            *object.ctv_to_tx.keys().next().unwrap()
        );
        for (control, (script, version)) in &input.tap_scripts {
            assert_eq!(*version, LeafVersion::TapScript);
            assert!(raw.leaves().iter().any(|(_, leaf)| leaf == script));
            assert!(control.verify_taproot_commitment(
                &Secp256k1::verification_only(),
                raw.spend_info().output_key().to_x_only_public_key(),
                script,
            ));
        }
        assert_eq!(signer.calls.load(Ordering::SeqCst), 1);
        assert_eq!(index.writes.get(), 1);
    }
}

#[test]
fn raw_artifacts_preserve_descriptor_and_funding_checks_before_signing() {
    for case in 0..3 {
        let mut object = artifact(false);
        let mut previous = funding(&object);
        match case {
            0 => {
                object.address =
                    sapio::util::extended_address::ExtendedAddress::Unknown(ScriptBuf::from(vec![
                        0x51,
                    ]));
            }
            1 => previous.output[0].script_pubkey = ScriptBuf::new(),
            2 => previous.output[0].value = bitcoin::Amount::from_sat(999),
            _ => unreachable!(),
        }
        let outpoint = OutPoint::new(previous.compute_txid(), 0);
        let index = Index::with(previous);
        let signer = Signer::default();
        let error = object
            .bind_psbt(outpoint, BTreeMap::new(), index.clone(), &signer)
            .unwrap_err();
        if case == 0 {
            let ObjectError::InvalidArtifact(error) = error else {
                panic!("unexpected error: {error}");
            };
            assert_eq!(error.kind, ArtifactErrorKind::DescriptorMismatch);
            assert_eq!(index.lookups.get(), 0);
        } else {
            assert!(matches!(error, ObjectError::InvalidFunding { .. }));
        }
        assert_eq!(signer.calls.load(Ordering::SeqCst), 0);
        assert_eq!(index.writes.get(), 0);
    }
}

#[test]
fn a_signer_cannot_strip_raw_spending_data_before_indexing() {
    let object = artifact(false);
    let previous = funding(&object);
    let outpoint = OutPoint::new(previous.compute_txid(), 0);
    let index = Index::with(previous);
    let signer = Signer {
        strip_scripts: true,
        ..Default::default()
    };
    assert!(matches!(
        object.bind_psbt(outpoint, BTreeMap::new(), index.clone(), &signer),
        Err(ObjectError::Emulator(EmulatorError::InvalidResponse))
    ));
    assert_eq!(signer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(index.writes.get(), 0);
}

#[test]
fn structured_guard_metadata_round_trips_without_stringifying_policy_sources() {
    let mut object = artifact(false);
    let policy = ScriptPolicy::And(vec![
        Clause::After(sapio::miniscript::AbsLockTime::from_consensus(100).unwrap()).into(),
        ScriptFragment::new(ScriptBuf::from(vec![0x51]))
            .unwrap()
            .into(),
    ]);
    object.metadata.simps_for_guards.push(GuardMetadata {
        policy: policy.clone(),
        protocols: BTreeMap::from([(-17, vec![serde_json::json!({"label": "raw guard"})])]),
    });
    let serialized = serde_json::to_value(&object).unwrap();
    assert_eq!(
        serialized["metadata"]["simps_for_guards"][0]["policy"],
        serde_json::to_value(policy).unwrap()
    );
    let decoded: Compiled = serde_json::from_value(serialized).unwrap();
    assert_eq!(decoded.metadata, object.metadata);
}
