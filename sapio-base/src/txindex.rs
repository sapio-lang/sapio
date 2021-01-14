use bitcoin::hash_types::*;
use bitcoincore_rpc_async as rpc;
use rpc::RpcApi;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Debug)]
pub enum TxIndexError {
    NetworkError(std::io::Error),
    UnknownTxid(Txid),
    IndexTooHigh(u32),
    RpcError(rpc::Error),
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
    cache: Cache,
    primary: Primary,
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

pub struct BitcoinNodeIndex {
    client: rpc::Client,
    runtime: tokio::runtime::Runtime,
    can_add: bool,
}

impl TxIndex for BitcoinNodeIndex {
    fn lookup_tx(&self, b: &Txid) -> Result<Arc<bitcoin::Transaction>> {
        tokio::task::block_in_place(|| {
            self.runtime
                .block_on(self.client.get_raw_transaction(b, None))
                .map(Arc::new)
                .map_err(TxIndexError::RpcError)
        })
    }
    fn add_tx(&self, tx: Arc<bitcoin::Transaction>) -> Result<Txid> {
        let txid = tx.txid();
        if self.can_add {
            tokio::task::block_in_place(|| {
                self.runtime
                    .block_on(self.client.send_raw_transaction(&*tx))
                    .map_err(TxIndexError::RpcError)
            })
        } else {
            Ok(txid)
        }
    }
}
