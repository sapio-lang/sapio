// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use bitcoin::hash_types::*;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;

/// Errors in resolving a TXIndex
#[derive(Debug)]
pub enum TxIndexError {
    /// Network Error
    NetworkError(std::io::Error),
    /// TXID Could not be resolved
    UnknownTxid(Txid),
    /// TXID exists, but the vout index was too high
    IndexTooHigh(u32),
    /// An index returned a transaction or acknowledgement for another TXID.
    TxidMismatch {
        /// The requested transaction ID.
        expected: Txid,
        /// The transaction ID returned by the index.
        actual: Txid,
    },
    /// Error in the Rpc System
    RpcError(Box<dyn std::error::Error>),
}
impl std::error::Error for TxIndexError {}

impl std::fmt::Display for TxIndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}
type Result<T> = std::result::Result<T, TxIndexError>;

fn check_txid(expected: Txid, actual: Txid) -> Result<()> {
    if expected != actual {
        return Err(TxIndexError::TxidMismatch { expected, actual });
    }
    Ok(())
}

/// Generic interface for any txindex
pub trait TxIndex {
    /// Look up a transaction whose TXID must match the requested ID.
    fn lookup_tx(&self, b: &Txid) -> Result<Arc<bitcoin::Transaction>>;
    /// lookup a particular output
    fn lookup_output(&self, b: &bitcoin::OutPoint) -> Result<bitcoin::TxOut> {
        let tx = self.lookup_tx(&b.txid)?;
        check_txid(b.txid, tx.txid())?;
        tx.output
            .get(b.vout as usize)
            .cloned()
            .ok_or(TxIndexError::IndexTooHigh(b.vout))
    }
    /// Add a transaction for tracking and return its TXID.
    fn add_tx(&self, tx: Arc<bitcoin::Transaction>) -> Result<Txid>;
}

/// a TxIndex which just tracks what it's seen and has no network
pub struct TxIndexLogger {
    map: Mutex<BTreeMap<Txid, Arc<bitcoin::Transaction>>>,
}
impl TxIndexLogger {
    /// create a new default instance
    pub fn new() -> TxIndexLogger {
        Self::default()
    }
}

impl Default for TxIndexLogger {
    fn default() -> Self {
        TxIndexLogger {
            map: Mutex::new(BTreeMap::new()),
        }
    }
}
impl TxIndex for TxIndexLogger {
    fn lookup_tx(&self, b: &Txid) -> Result<Arc<bitcoin::Transaction>> {
        self.map
            .lock()
            .unwrap()
            .get(b)
            .cloned()
            .ok_or(TxIndexError::UnknownTxid(*b))
    }
    fn add_tx(&self, tx: Arc<bitcoin::Transaction>) -> Result<Txid> {
        let txid = tx.txid();
        self.map.lock().unwrap().insert(txid, tx);
        Ok(txid)
    }
}

/// a cached txindex checks a cache first and then a primary txindex
///
/// Only an unknown requested TXID permits fallback. Identical transactions
/// are deduplicated on insertion; changed witnesses are sent to the primary
/// before replacing the cached transaction.
pub struct CachedTxIndex<Cache: TxIndex, Primary: TxIndex> {
    /// the cache txindex
    pub cache: Cache,
    /// the main txindex
    pub primary: Primary,
}

impl<Cache, Primary> TxIndex for CachedTxIndex<Cache, Primary>
where
    Cache: TxIndex,
    Primary: TxIndex,
{
    fn lookup_tx(&self, b: &Txid) -> Result<Arc<bitcoin::Transaction>> {
        match self.cache.lookup_tx(b) {
            Ok(tx) => {
                check_txid(*b, tx.txid())?;
                Ok(tx)
            }
            Err(TxIndexError::UnknownTxid(txid)) if txid == *b => {
                let tx = self.primary.lookup_tx(b)?;
                check_txid(*b, tx.txid())?;
                check_txid(*b, self.cache.add_tx(tx.clone())?)?;
                Ok(tx)
            }
            Err(error) => Err(error),
        }
    }
    fn add_tx(&self, tx: Arc<bitcoin::Transaction>) -> Result<Txid> {
        let txid = tx.txid();
        match self.cache.lookup_tx(&txid) {
            Ok(cached) => {
                check_txid(txid, cached.txid())?;
                if cached == tx {
                    return Ok(txid);
                }
            }
            Err(TxIndexError::UnknownTxid(missing)) if missing == txid => {}
            Err(error) => return Err(error),
        }
        check_txid(txid, self.primary.add_tx(tx.clone())?)?;
        check_txid(txid, self.cache.add_tx(tx)?)?;
        Ok(txid)
    }
}
