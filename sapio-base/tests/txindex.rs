use bitcoin::{OutPoint, Script, Transaction, TxIn, TxOut, Txid, Witness};
use sapio_base::txindex::{CachedTxIndex, TxIndex, TxIndexError, TxIndexLogger};
use std::cell::RefCell;
use std::io;
use std::sync::Arc;

type Lookup = Result<Arc<Transaction>, TxIndexError>;

#[derive(Default)]
struct StubIndex {
    lookup: RefCell<Option<Lookup>>,
    add: RefCell<Option<Result<Txid, TxIndexError>>>,
    requested: RefCell<Vec<Txid>>,
    added: RefCell<Vec<Arc<Transaction>>>,
}

impl StubIndex {
    fn with_lookup(self, result: Lookup) -> Self {
        *self.lookup.borrow_mut() = Some(result);
        self
    }

    fn with_add(self, result: Result<Txid, TxIndexError>) -> Self {
        *self.add.borrow_mut() = Some(result);
        self
    }

    fn assert_untouched(&self) {
        assert!(self.requested.borrow().is_empty());
        assert!(self.added.borrow().is_empty());
    }
}

impl TxIndex for StubIndex {
    fn lookup_tx(&self, txid: &Txid) -> Lookup {
        self.requested.borrow_mut().push(*txid);
        self.lookup.borrow_mut().take().expect("unexpected lookup")
    }

    fn add_tx(&self, tx: Arc<Transaction>) -> Result<Txid, TxIndexError> {
        self.added.borrow_mut().push(tx);
        self.add.borrow_mut().take().expect("unexpected add")
    }
}

fn transaction(value: u64) -> Arc<Transaction> {
    Arc::new(Transaction {
        version: 2,
        lock_time: 0,
        input: vec![TxIn::default()],
        output: vec![TxOut {
            value,
            script_pubkey: Script::new(),
        }],
    })
}

fn assert_mismatch(error: TxIndexError, expected: Txid, actual: Txid) {
    assert!(
        matches!(error, TxIndexError::TxidMismatch { expected: e, actual: a }
            if e == expected && a == actual),
        "{error}"
    );
}

fn failures(other: Txid) -> Vec<TxIndexError> {
    vec![
        TxIndexError::NetworkError(io::Error::new(io::ErrorKind::TimedOut, "offline")),
        TxIndexError::RpcError(Box::new(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "RPC unavailable",
        ))),
        TxIndexError::UnknownTxid(other),
    ]
}

fn assert_same_error(actual: TxIndexError, expected: &str) {
    assert_eq!(actual.to_string(), expected);
}

#[test]
fn output_lookup_checks_identity_before_the_output_index() {
    let expected = transaction(42).txid();
    let wrong = transaction(43);
    for vout in [0, 1] {
        let index = StubIndex::default().with_lookup(Ok(wrong.clone()));
        let error = index
            .lookup_output(&OutPoint {
                txid: expected,
                vout,
            })
            .unwrap_err();
        assert_mismatch(error, expected, wrong.txid());
    }

    let tx = transaction(42);
    let index = TxIndexLogger::new();
    assert!(
        matches!(index.lookup_tx(&expected), Err(TxIndexError::UnknownTxid(id)) if id == expected)
    );
    index.add_tx(tx.clone()).unwrap();
    assert_eq!(
        index
            .lookup_output(&OutPoint {
                txid: expected,
                vout: 0
            })
            .unwrap(),
        tx.output[0]
    );
    assert!(matches!(
        index.lookup_output(&OutPoint {
            txid: expected,
            vout: 1
        }),
        Err(TxIndexError::IndexTooHigh(1))
    ));
}

#[test]
fn cache_miss_loads_once_and_subsequent_hit_skips_the_primary() {
    let tx = transaction(42);
    let index = CachedTxIndex {
        cache: TxIndexLogger::new(),
        primary: StubIndex::default().with_lookup(Ok(tx.clone())),
    };
    assert_eq!(index.lookup_tx(&tx.txid()).unwrap(), tx);
    assert_eq!(index.lookup_tx(&tx.txid()).unwrap(), tx);
    assert_eq!(*index.primary.requested.borrow(), vec![tx.txid()]);
    assert!(index.primary.added.borrow().is_empty());
}

#[test]
fn cache_errors_do_not_fall_back_during_lookup_or_add() {
    let tx = transaction(42);
    for adding in [false, true] {
        for error in failures(transaction(43).txid()) {
            let expected = error.to_string();
            let index = CachedTxIndex {
                cache: StubIndex::default().with_lookup(Err(error)),
                primary: StubIndex::default(),
            };
            let error = if adding {
                index.add_tx(tx.clone()).unwrap_err()
            } else {
                index.lookup_tx(&tx.txid()).unwrap_err()
            };
            assert_same_error(error, &expected);
            index.primary.assert_untouched();
            assert!(index.cache.added.borrow().is_empty());
        }
    }
}

#[test]
fn wrong_cache_transactions_stop_lookup_and_add() {
    let tx = transaction(42);
    let wrong = transaction(43);
    for adding in [false, true] {
        let index = CachedTxIndex {
            cache: StubIndex::default().with_lookup(Ok(wrong.clone())),
            primary: StubIndex::default(),
        };
        let error = if adding {
            index.add_tx(tx.clone()).unwrap_err()
        } else {
            index.lookup_tx(&tx.txid()).unwrap_err()
        };
        assert_mismatch(error, tx.txid(), wrong.txid());
        index.primary.assert_untouched();
        assert!(index.cache.added.borrow().is_empty());
    }
}

#[test]
fn wrong_primary_transactions_never_enter_the_cache() {
    let expected = transaction(42).txid();
    let wrong = transaction(43);
    let index = CachedTxIndex {
        cache: StubIndex::default().with_lookup(Err(TxIndexError::UnknownTxid(expected))),
        primary: StubIndex::default().with_lookup(Ok(wrong.clone())),
    };
    assert_mismatch(
        index.lookup_tx(&expected).unwrap_err(),
        expected,
        wrong.txid(),
    );
    assert!(index.cache.added.borrow().is_empty());
}

#[test]
fn primary_lookup_errors_leave_the_cache_untouched() {
    let txid = transaction(42).txid();
    for error in failures(txid) {
        let expected = error.to_string();
        let index = CachedTxIndex {
            cache: StubIndex::default().with_lookup(Err(TxIndexError::UnknownTxid(txid))),
            primary: StubIndex::default().with_lookup(Err(error)),
        };
        assert_same_error(index.lookup_tx(&txid).unwrap_err(), &expected);
        assert!(index.cache.added.borrow().is_empty());
    }
}

#[test]
fn lookup_rejects_an_incorrect_cache_add_acknowledgement() {
    let tx = transaction(42);
    let wrong = transaction(43).txid();
    let index = CachedTxIndex {
        cache: StubIndex::default()
            .with_lookup(Err(TxIndexError::UnknownTxid(tx.txid())))
            .with_add(Ok(wrong)),
        primary: StubIndex::default().with_lookup(Ok(tx.clone())),
    };
    assert_mismatch(index.lookup_tx(&tx.txid()).unwrap_err(), tx.txid(), wrong);
}

#[test]
fn add_miss_populates_primary_then_cache() {
    let tx = transaction(42);
    let index = CachedTxIndex {
        cache: TxIndexLogger::new(),
        primary: StubIndex::default().with_add(Ok(tx.txid())),
    };
    assert_eq!(index.add_tx(tx.clone()).unwrap(), tx.txid());
    assert_eq!(index.lookup_tx(&tx.txid()).unwrap(), tx);
    assert_eq!(*index.primary.added.borrow(), vec![tx]);
    assert!(index.primary.requested.borrow().is_empty());
}

#[test]
fn incorrect_primary_add_acknowledgements_never_populate_the_cache() {
    let tx = transaction(42);
    let wrong = transaction(43).txid();
    let index = CachedTxIndex {
        cache: StubIndex::default().with_lookup(Err(TxIndexError::UnknownTxid(tx.txid()))),
        primary: StubIndex::default().with_add(Ok(wrong)),
    };
    assert_mismatch(index.add_tx(tx.clone()).unwrap_err(), tx.txid(), wrong);
    assert!(index.cache.added.borrow().is_empty());
}

#[test]
fn add_rejects_an_incorrect_cache_acknowledgement() {
    let tx = transaction(42);
    let wrong = transaction(43).txid();
    let index = CachedTxIndex {
        cache: StubIndex::default()
            .with_lookup(Err(TxIndexError::UnknownTxid(tx.txid())))
            .with_add(Ok(wrong)),
        primary: StubIndex::default().with_add(Ok(tx.txid())),
    };
    assert_mismatch(index.add_tx(tx.clone()).unwrap_err(), tx.txid(), wrong);
}

#[test]
fn primary_add_errors_do_not_populate_the_cache() {
    let tx = transaction(42);
    for error in failures(tx.txid()) {
        let expected = error.to_string();
        let index = CachedTxIndex {
            cache: StubIndex::default().with_lookup(Err(TxIndexError::UnknownTxid(tx.txid()))),
            primary: StubIndex::default().with_add(Err(error)),
        };
        assert_same_error(index.add_tx(tx.clone()).unwrap_err(), &expected);
        assert!(index.cache.added.borrow().is_empty());
    }
}

#[test]
fn cache_add_errors_propagate_during_lookup_and_add() {
    let tx = transaction(42);
    for adding in [false, true] {
        for error in failures(tx.txid()) {
            let expected = error.to_string();
            let index = CachedTxIndex {
                cache: StubIndex::default()
                    .with_lookup(Err(TxIndexError::UnknownTxid(tx.txid())))
                    .with_add(Err(error)),
                primary: StubIndex::default()
                    .with_lookup(Ok(tx.clone()))
                    .with_add(Ok(tx.txid())),
            };
            let error = if adding {
                index.add_tx(tx.clone()).unwrap_err()
            } else {
                index.lookup_tx(&tx.txid()).unwrap_err()
            };
            assert_same_error(error, &expected);
        }
    }
}

#[test]
fn identical_adds_do_not_rewrite_either_index() {
    let tx = transaction(42);
    let index = CachedTxIndex {
        cache: StubIndex::default().with_lookup(Ok(tx.clone())),
        primary: StubIndex::default(),
    };
    assert_eq!(index.add_tx(tx.clone()).unwrap(), tx.txid());
    assert!(index.cache.added.borrow().is_empty());
    index.primary.assert_untouched();
}

#[test]
fn changed_witnesses_update_both_indexes_but_identical_adds_are_deduplicated() {
    let original = transaction(42);
    let mut signed = (*original).clone();
    signed.input[0].witness = Witness::from_vec(vec![vec![1; 64]]);
    let signed = Arc::new(signed);
    assert_eq!(original.txid(), signed.txid());
    assert_ne!(original.wtxid(), signed.wtxid());

    let index = CachedTxIndex {
        cache: TxIndexLogger::new(),
        primary: StubIndex::default().with_add(Ok(signed.txid())),
    };
    index.cache.add_tx(original.clone()).unwrap();
    assert_eq!(index.add_tx(original.clone()).unwrap(), original.txid());
    index.primary.assert_untouched();

    assert_eq!(index.add_tx(signed.clone()).unwrap(), signed.txid());
    assert_eq!(*index.primary.added.borrow(), vec![signed.clone()]);
    assert_eq!(index.lookup_tx(&signed.txid()).unwrap(), signed);

    assert_eq!(index.add_tx(signed.clone()).unwrap(), signed.txid());
    assert_eq!(*index.primary.added.borrow(), vec![signed.clone()]);
    assert_eq!(index.lookup_tx(&signed.txid()).unwrap(), signed);
}
