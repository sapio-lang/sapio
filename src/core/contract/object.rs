use crate::clause::Clause;
use crate::txn::Template;
use crate::util::amountrange::AmountRange;
use ::miniscript::*;
use bitcoin::hashes::sha256;
use bitcoin::util::amount::Amount;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
/// Object holds a contract's complete context required post-compilation
/// There is no guarantee that Object is properly constructed presently.
//TODO: Make type immutable and correct by construction...
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Object {
    pub ctv_to_tx: HashMap<sha256::Hash, Template>,
    pub suggested_txs: HashMap<sha256::Hash, Template>,
    pub policy: Option<Clause>,
    pub address: bitcoin::Address,
    pub descriptor: Option<Descriptor<bitcoin::PublicKey>>,
    pub amount_range: AmountRange,
}

impl Object {
    /// converts a descriptor and an optional AmountRange to a Object object.
    /// This can be used for e.g. creating raw SegWit Scripts.
    pub fn from_descriptor(d: Descriptor<bitcoin::PublicKey>, a: Option<AmountRange>) -> Object {
        Object {
            ctv_to_tx: HashMap::new(),
            suggested_txs: HashMap::new(),
            policy: None,
            address: d.address(bitcoin::Network::Bitcoin).unwrap(),
            descriptor: Some(d),
            amount_range: a.unwrap_or_else(|| {
                let mut a = AmountRange::new();
                a.update_range(Amount::min_value());
                a.update_range(Amount::max_value());
                a
            }),
        }
    }

    pub fn from_address(address: bitcoin::Address, a: Option<AmountRange>) -> Object {
        Object {
            ctv_to_tx: HashMap::new(),
            suggested_txs: HashMap::new(),
            policy: None,
            address,
            descriptor: None,
            amount_range: a.unwrap_or_else(|| {
                let mut a = AmountRange::new();
                a.update_range(Amount::min_value());
                a.update_range(Amount::max_value());
                a
            }),
        }
    }

    pub fn bind(
        &self,
        out_in: bitcoin::OutPoint,
    ) -> (Vec<bitcoin::Transaction>, Vec<serde_json::Value>) {
        let mut txns = vec![];
        let mut metadata_out = vec![];
        // Could use a queue instead to do BFS linking, but order doesn't matter and stack is
        // faster.
        let mut stack = vec![(out_in, self)];

        while let Some((
            out,
            Object {
                descriptor,
                ctv_to_tx,
                suggested_txs,
                ..
            },
        )) = stack.pop()
        {
            txns.reserve(ctv_to_tx.len() + suggested_txs.len());
            metadata_out.reserve(ctv_to_tx.len() + suggested_txs.len());
            for (
                _ctv_hash,
                Template {
                    label, outputs, tx, ..
                },
            ) in ctv_to_tx.iter().chain(suggested_txs.iter())
            {
                let mut tx = tx.clone();
                tx.input[0].previous_output = out;
                // Missing other Witness Info.
                if let Some(d) = descriptor {
                    tx.input[0].witness = vec![d.witness_script().into_bytes()];
                }
                let txid = tx.txid();
                txns.push(tx);
                metadata_out.push(json!({
                    "color" : "green",
                    "label" : label,
                    "utxo_metadata" : outputs.iter().map(|x| &x.metadata).collect::<Vec<_>>()
                }));
                stack.reserve(outputs.len());
                for (vout, v) in outputs.iter().enumerate() {
                    let vout = vout as u32;
                    stack.push((bitcoin::OutPoint { txid, vout }, &v.contract));
                }
            }
        }
        (txns, metadata_out)
    }
}
