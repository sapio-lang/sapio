// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use bitcoin::hash_types::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Debug)]
pub enum TxIndexError {
    NetworkError(std::io::Error),
    UnknownTxid(Txid),
    IndexTooHigh(u32),
    RpcError(Box<dyn std::error::Error>),
}
impl std::error::Error for TxIndexError {}

impl std::fmt::Display for TxIndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}
type Result<T> = std::result::Result<T, TxIndexError>;
pub trait TxIndex {
    fn lookup_tx(&self, b: &Txid) -> Result<Arc<bitcoin::Transaction>>;
    fn lookup_output(&self, b: &bitcoin::OutPoint) -> Result<bitcoin::TxOut> {
        self.lookup_tx(&b.txid)?
            .output
            .get(b.vout as usize)
            .cloned()
            .ok_or(TxIndexError::IndexTooHigh(b.vout))
    }
    fn add_tx(&self, tx: Arc<bitcoin::Transaction>) -> Result<Txid>;
}
pub struct TxIndexLogger {
    map: Mutex<HashMap<Txid, Arc<bitcoin::Transaction>>>,
}
impl TxIndexLogger {
    pub fn new() -> TxIndexLogger {
        TxIndexLogger {
            map: Mutex::new(HashMap::new()),
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
            .ok_or_else(|| TxIndexError::UnknownTxid(*b))
    }
    fn add_tx(&self, tx: Arc<bitcoin::Transaction>) -> Result<Txid> {
        let txid = tx.txid();
        self.map.lock().unwrap().insert(txid, tx);
        Ok(txid)
    }
}

pub struct CachedTxIndex<Cache: TxIndex, Primary: TxIndex> {
    pub cache: Cache,
    pub primary: Primary,
}

impl<Cache, Primary> TxIndex for CachedTxIndex<Cache, Primary>
where
    Cache: TxIndex,
    Primary: TxIndex,
{
    fn lookup_tx(&self, b: &Txid) -> Result<Arc<bitcoin::Transaction>> {
        if let Ok(ent) = self.cache.lookup_tx(b) {
            Ok(ent)
        } else {
            let ent = self.primary.lookup_tx(&b)?;
            self.cache.add_tx(ent.clone())?;
            Ok(ent)
        }
    }
    fn add_tx(&self, tx: Arc<bitcoin::Transaction>) -> Result<Txid> {
        let txid = tx.txid();
        if self.cache.lookup_tx(&txid).is_ok() {
            Ok(txid)
        } else {
            self.primary.add_tx(tx.clone())?;
            self.cache.add_tx(tx)
        }
    }
}
