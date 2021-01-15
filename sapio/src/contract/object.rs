use crate::template::Template;
use crate::util::amountrange::AmountRange;
use ::miniscript::{self, *};

use bitcoin::hashes::sha256;
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::util::amount::Amount;
use bitcoin::util::psbt::PartiallySignedTransaction;
use sapio_ctv_emulator_trait::{CTVEmulator, NullEmulator, EmulatorError};
use sapio_base::txindex::TxIndexError;
use sapio_base::txindex::{TxIndex, TxIndexLogger};
use sapio_base::Clause;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Debug)]
pub enum ObjectError {
    Miniscript(miniscript::policy::compiler::CompilerError),
    Custom(Box<dyn std::error::Error>),
}
impl std::error::Error for ObjectError {}
impl From<EmulatorError> for ObjectError {
    fn from(e: EmulatorError) -> Self {
        ObjectError::Custom(Box::new(e))
    }
}
impl From<TxIndexError> for ObjectError {
    fn from(e: TxIndexError) -> Self {
        ObjectError::Custom(Box::new(e))
    }
}

impl From<miniscript::policy::compiler::CompilerError> for ObjectError {
    fn from(v: miniscript::policy::compiler::CompilerError) -> Self {
        ObjectError::Miniscript(v)
    }
}

impl std::fmt::Display for ObjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}
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
    /// Creates an object from a given address. The optional AmountRange argument determines the
    /// safe bounds the contract can receive, otherwise it is set to any.
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

    /// bind attaches an Object to a specific UTXO and returns a vec of transactions and
    /// transaction metadata.
    ///
    pub fn bind(
        &self,
        out_in: bitcoin::OutPoint,
    ) -> (Vec<bitcoin::Transaction>, Vec<serde_json::Value>) {
        let (a, b) = self
            .bind_psbt(
                out_in,
                HashMap::new(),
                Rc::new(TxIndexLogger::new()),
                &NullEmulator(None),
            )
            .unwrap();
        (a.into_iter().map(|x| x.extract_tx()).collect(), b)
    }
    pub fn bind_psbt(
        &self,
        out_in: bitcoin::OutPoint,
        output_map: HashMap<Sha256, Vec<Option<bitcoin::OutPoint>>>,
        blockdata: Rc<dyn TxIndex>,
        emulator: &dyn CTVEmulator,
    ) -> Result<
        (
            Vec<bitcoin::util::psbt::PartiallySignedTransaction>,
            Vec<serde_json::Value>,
        ),
        ObjectError,
    > {
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
                ctv_hash,
                Template {
                    label, outputs, tx, ..
                },
            ) in ctv_to_tx.iter().chain(suggested_txs.iter())
            {
                let mut tx = tx.clone();
                tx.input[0].previous_output = out;
                if let Some(outputs) = output_map.get(ctv_hash) {
                    for (i, inp) in tx.input.iter_mut().enumerate().skip(1) {
                        if let Some(out) = outputs[i] {
                            inp.previous_output = out;
                        }
                    }
                }
                let mut psbtx = PartiallySignedTransaction::from_unsigned_tx(tx.clone()).unwrap();
                for (psbt_in, tx_in) in psbtx.inputs.iter_mut().zip(tx.input.iter()) {
                    psbt_in.witness_utxo = blockdata.lookup_output(&tx_in.previous_output).ok();
                    psbt_in.sighash_type = Some(bitcoin::blockdata::transaction::SigHashType::All);
                }
                // Missing other Witness Info.
                if let Some(d) = descriptor {
                    psbtx.inputs[0].witness_script = Some(d.explicit_script());
                }
                psbtx = emulator.sign(psbtx)?;
                let final_tx = psbtx.clone().extract_tx();
                let txid = blockdata.add_tx(Arc::new(final_tx))?;
                txns.push(psbtx);
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
        Ok((txns, metadata_out))
    }
}
