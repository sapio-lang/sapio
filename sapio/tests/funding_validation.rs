use bitcoin::hashes::{sha256, Hash};
use bitcoin::psbt::PartiallySignedTransaction as Psbt;
use bitcoin::{Address, Amount, Network, OutPoint, Script, Transaction, TxIn, TxOut, Txid};
use sapio::contract::abi::object::ObjectError;
use sapio::contract::abi::studio::{Program, SapioStudioFormat};
use sapio::contract::{Compilable, Compiled, Context, Contract};
use sapio::{declare, then};
use sapio_base::txindex::{TxIndex, TxIndexError};
use sapio_base::Clause;
use sapio_ctv_emulator_trait::{CTVAvailable, CTVEmulator, EmulatorError};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct Payment {
    destination: Compiled,
    extra_input: bool,
    fees: u64,
}

impl Payment {
    #[then]
    fn pay(self, ctx: Context) {
        let mut builder =
            ctx.template()
                .add_output(Amount::from_sat(1_000), &self.destination, None)?;
        if self.extra_input {
            builder = builder.add_sequence();
        }
        builder.add_fees(Amount::from_sat(self.fees))?.into()
    }
}

impl Contract for Payment {
    declare! {then, Self::pay}
    declare! {non updatable}
}

fn leaf() -> Compiled {
    Compiled::from_address(
        Address::from_str("bcrt1qumrrqgt7e3a7damzm8x97m6sjs20u8hjw2hcjj").unwrap(),
        None,
    )
}

fn payment(destination: Compiled, extra_input: bool, fees: u64, path: &str) -> Compiled {
    Payment {
        destination,
        extra_input,
        fees,
    }
    .compile(Context::new(
        Network::Regtest,
        Amount::from_sat(1_000 + fees),
        Arc::new(CTVAvailable),
        path.try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    ))
    .unwrap()
}

fn funding(object: &Compiled, value: u64) -> Transaction {
    Transaction {
        version: 2,
        lock_time: 0,
        input: vec![TxIn::default()],
        output: vec![TxOut {
            value,
            script_pubkey: object.address.clone().into(),
        }],
    }
}

#[derive(Default)]
struct Index {
    txs: RefCell<BTreeMap<Txid, Arc<Transaction>>>,
    writes: Cell<usize>,
    fail_lookup: Cell<bool>,
    wrong_ack: Cell<bool>,
}

impl Index {
    fn with(tx: Transaction) -> Rc<Self> {
        let index = Rc::new(Self::default());
        index.txs.borrow_mut().insert(tx.txid(), Arc::new(tx));
        index
    }
}

impl TxIndex for Index {
    fn lookup_tx(&self, txid: &Txid) -> Result<Arc<Transaction>, TxIndexError> {
        if self.fail_lookup.get() {
            return Err(TxIndexError::NetworkError(std::io::Error::other(
                "unavailable",
            )));
        }
        self.txs
            .borrow()
            .get(txid)
            .cloned()
            .ok_or(TxIndexError::UnknownTxid(*txid))
    }

    fn add_tx(&self, tx: Arc<Transaction>) -> Result<Txid, TxIndexError> {
        self.writes.set(self.writes.get() + 1);
        if self.wrong_ack.get() {
            return Ok(Txid::from_slice(&[0; 32]).unwrap());
        }
        let txid = tx.txid();
        self.txs.borrow_mut().insert(txid, tx);
        Ok(txid)
    }
}

#[derive(Default)]
struct Signer {
    calls: AtomicUsize,
    corrupt_on: Option<usize>,
}

impl CTVEmulator for Signer {
    fn get_signer_for(&self, _: sha256::Hash) -> Result<Clause, EmulatorError> {
        unreachable!("binding does not compile signer policies")
    }

    fn sign(&self, mut psbt: Psbt) -> Result<Psbt, EmulatorError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.corrupt_on == Some(call) {
            psbt.inputs[0].witness_utxo.as_mut().unwrap().value += 1;
        }
        Ok(psbt)
    }
}

fn psbt(program: &Program, object: &Compiled) -> Psbt {
    let SapioStudioFormat::LinkedPSBT { psbt, .. } =
        &program.program.get(&object.root_path).unwrap().txs[0];
    bitcoin::consensus::deserialize(&base64::decode(psbt).unwrap()).unwrap()
}

#[test]
fn authenticates_funding_transactions_and_propagates_lookup_errors() {
    let object = payment(leaf(), false, 100, "payment");
    for case in 0..3 {
        let tx = funding(&object, 1_100);
        let mut out = OutPoint::new(tx.txid(), 0);
        let index = Index::with(tx.clone());
        match case {
            0 => {
                let mut wrong = tx;
                wrong.output[0].value += 1;
                index.txs.borrow_mut().insert(out.txid, Arc::new(wrong));
            }
            1 => out.vout = 1,
            2 => index.fail_lookup.set(true),
            _ => unreachable!(),
        }
        let signer = Signer::default();
        assert!(
            object
                .bind_psbt(out, BTreeMap::new(), index.clone(), &signer)
                .is_err(),
            "case {case}"
        );
        assert_eq!(signer.calls.load(Ordering::SeqCst), 0);
        assert_eq!(index.writes.get(), 0);
    }
}

#[test]
fn unrelated_unknown_txid_is_an_error_before_signing_or_indexing() {
    struct UnrelatedMiss(Txid);

    impl TxIndex for UnrelatedMiss {
        fn lookup_tx(&self, _: &Txid) -> Result<Arc<Transaction>, TxIndexError> {
            Err(TxIndexError::UnknownTxid(self.0))
        }

        fn add_tx(&self, _: Arc<Transaction>) -> Result<Txid, TxIndexError> {
            panic!("an unrelated lookup miss must stop before index writes")
        }
    }

    let object = payment(leaf(), false, 100, "payment");
    let requested = funding(&object, 1_100).txid();
    let unrelated = funding(&object, 1_101).txid();
    assert_ne!(requested, unrelated);
    let signer = Signer::default();
    let result = object.bind_psbt(
        OutPoint::new(requested, 0),
        BTreeMap::new(),
        Rc::new(UnrelatedMiss(unrelated)),
        &signer,
    );
    assert!(matches!(
        result,
        Err(ObjectError::TxIndex(TxIndexError::UnknownTxid(txid))) if txid == unrelated
    ));
    assert_eq!(signer.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn authenticates_known_funding_without_calling_output_lookup_overrides() {
    struct OutputOverride(Rc<Index>);

    impl TxIndex for OutputOverride {
        fn lookup_tx(&self, txid: &Txid) -> Result<Arc<Transaction>, TxIndexError> {
            self.0.lookup_tx(txid)
        }

        fn lookup_output(&self, _: &OutPoint) -> Result<TxOut, TxIndexError> {
            panic!("binding must authenticate whole previous transactions")
        }

        fn add_tx(&self, tx: Arc<Transaction>) -> Result<Txid, TxIndexError> {
            self.0.add_tx(tx)
        }
    }

    let object = payment(leaf(), false, 100, "payment");
    let tx = funding(&object, 1_100);
    let out = OutPoint::new(tx.txid(), 0);
    let program = object
        .bind_psbt(
            out,
            BTreeMap::new(),
            Rc::new(OutputOverride(Index::with(tx.clone()))),
            &CTVAvailable,
        )
        .unwrap();
    let bound = psbt(&program, &object);
    assert_eq!(bound.unsigned_tx.input[0].previous_output, out);
    assert_eq!(bound.inputs[0].non_witness_utxo, Some(tx.clone()));
    assert_eq!(bound.inputs[0].witness_utxo, Some(tx.output[0].clone()));
}

#[test]
fn checks_contract_script_and_reserved_fees_before_signing() {
    let object = payment(leaf(), false, 100, "payment");
    for wrong_script in [false, true] {
        let mut tx = funding(&object, if wrong_script { 1_100 } else { 1_099 });
        if wrong_script {
            tx.output[0].script_pubkey = Script::new();
        }
        let out = OutPoint::new(tx.txid(), 0);
        let index = Index::with(tx);
        let signer = Signer::default();
        assert!(object
            .bind_psbt(out, BTreeMap::new(), index.clone(), &signer)
            .is_err());
        assert_eq!(signer.calls.load(Ordering::SeqCst), 0);
        assert_eq!(index.writes.get(), 0);
    }
}

#[test]
fn authenticates_every_known_prevout_and_accepts_excess_funding() {
    let object = payment(leaf(), false, 100, "payment");
    let tx = funding(&object, 1_200);
    let out = OutPoint::new(tx.txid(), 0);
    let program = object
        .bind_psbt(out, BTreeMap::new(), Index::with(tx.clone()), &CTVAvailable)
        .unwrap();
    let bound = psbt(&program, &object);
    assert_eq!(bound.inputs[0].non_witness_utxo, Some(tx.clone()));
    assert_eq!(bound.inputs[0].witness_utxo, Some(tx.output[0].clone()));
    assert_eq!(bound.unsigned_tx.input[0].previous_output, out);
    assert_eq!(bound.unsigned_tx.output[0].value, 1_000);
}

#[test]
fn sums_auxiliary_funding_and_rejects_duplicates_and_overflow() {
    let object = payment(leaf(), true, 100, "payment");
    let hash = *object.ctv_to_tx.keys().next().unwrap();
    for (root_value, auxiliary_value, duplicate, valid) in [
        (400, 700, false, true),
        (400, 699, false, false),
        (1_100, 1_100, true, false),
        (u64::MAX, 1, false, false),
    ] {
        let tx = funding(&object, root_value);
        let out = OutPoint::new(tx.txid(), 0);
        let index = Index::with(tx);
        let auxiliary = funding(&leaf(), auxiliary_value);
        let auxiliary_out = OutPoint::new(auxiliary.txid(), 0);
        index
            .txs
            .borrow_mut()
            .insert(auxiliary.txid(), Arc::new(auxiliary));
        let mapping = BTreeMap::from([(
            hash,
            vec![None, Some(if duplicate { out } else { auxiliary_out })],
        )]);
        let signer = Signer::default();
        let result = object.bind_psbt(out, mapping, index.clone(), &signer);
        assert_eq!(
            result.is_ok(),
            valid,
            "{root_value} + {auxiliary_value}, duplicate {duplicate}"
        );
        if valid {
            let bound = psbt(&result.unwrap(), &object);
            assert_eq!(
                bound.inputs[1].witness_utxo.as_ref().unwrap().value,
                auxiliary_value
            );
            assert_eq!(
                bound.inputs[1].non_witness_utxo.as_ref().unwrap().txid(),
                auxiliary_out.txid
            );
        } else {
            assert_eq!(signer.calls.load(Ordering::SeqCst), 0);
            assert_eq!(index.writes.get(), 0);
        }
    }
}

#[test]
fn unresolved_inputs_remain_unsigned_and_do_not_collide_with_contract_input() {
    let object = payment(leaf(), true, 100, "payment");
    let out = OutPoint::new(Txid::from_slice(&[0; 32]).unwrap(), 0);
    let program = object
        .bind_psbt(
            out,
            BTreeMap::new(),
            Rc::new(Index::default()),
            &CTVAvailable,
        )
        .unwrap();
    let bound = psbt(&program, &object);
    assert_ne!(
        bound.unsigned_tx.input[0].previous_output,
        bound.unsigned_tx.input[1].previous_output
    );
    assert!(bound
        .inputs
        .iter()
        .all(|input| input.witness_utxo.is_none() && input.non_witness_utxo.is_none()));
}

#[test]
fn rejects_underfunded_descendants_before_any_signing_or_index_writes() {
    let child = payment(leaf(), false, 100, "child");
    let object = payment(child, false, 0, "parent");
    let tx = funding(&object, 1_000);
    let out = OutPoint::new(tx.txid(), 0);
    let index = Index::with(tx);
    let signer = Signer::default();
    assert!(object
        .bind_psbt(out, BTreeMap::new(), index.clone(), &signer)
        .is_err());
    assert_eq!(signer.calls.load(Ordering::SeqCst), 0);
    assert_eq!(index.writes.get(), 0);
}

#[test]
fn validates_all_signer_responses_before_indexing_the_graph() {
    let child = payment(leaf(), false, 0, "child");
    let object = payment(child, false, 0, "parent");
    for corrupt_on in [1, 2] {
        let tx = funding(&object, 1_000);
        let out = OutPoint::new(tx.txid(), 0);
        let index = Index::with(tx);
        let signer = Signer {
            corrupt_on: Some(corrupt_on),
            ..Default::default()
        };
        assert!(object
            .bind_psbt(out, BTreeMap::new(), index.clone(), &signer)
            .is_err());
        assert_eq!(signer.calls.load(Ordering::SeqCst), corrupt_on);
        assert_eq!(index.writes.get(), 0);
    }
}

#[test]
fn rejects_incorrect_index_acknowledgements() {
    let object = payment(leaf(), false, 0, "payment");
    let tx = funding(&object, 1_000);
    let out = OutPoint::new(tx.txid(), 0);
    let index = Index::with(tx);
    index.wrong_ack.set(true);
    assert!(object
        .bind_psbt(out, BTreeMap::new(), index.clone(), &CTVAvailable)
        .is_err());
    assert_eq!(index.writes.get(), 1);
}
