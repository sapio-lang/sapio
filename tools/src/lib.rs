// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! tools is a workspace-global set of util functions that require heavy dependencies

#![deny(missing_docs)]
use bitcoin::Txid;
use bitcoincore_rpc as rpc;
use rpc::RpcApi;
use sapio_base::txindex::{TxIndex, TxIndexError};
use std::sync::Arc;
/// A TxIndex based on a Bitcoin RPC Client
pub struct BitcoinNodeIndex {
    /// RPC Client
    pub client: rpc::Client,
    /// if can_add is true, then allow the Index to call send_raw_transaction
    pub can_add: bool,
}

type Result<T> = std::result::Result<T, TxIndexError>;
impl TxIndex for BitcoinNodeIndex {
    fn lookup_tx(&self, b: &Txid) -> Result<Arc<bitcoin::Transaction>> {
        self.client
            .get_raw_transaction(b, None)
            .map(Arc::new)
            .map_err(|e| {
                let b: Box<dyn std::error::Error> = Box::new(e);
                TxIndexError::RpcError(b)
            })
    }
    fn add_tx(&self, tx: Arc<bitcoin::Transaction>) -> Result<Txid> {
        let txid = tx.compute_txid();
        if self.can_add {
            self.client.send_raw_transaction(&*tx).map_err(|e| {
                let b: Box<dyn std::error::Error> = Box::new(e);
                TxIndexError::RpcError(b)
            })
        } else {
            Ok(txid)
        }
    }
}
