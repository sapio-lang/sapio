// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Object is the output of Sapio Compilation & can be linked to a specific coin
use crate::template::Template;
use crate::util::amountrange::AmountRange;
use crate::util::extended_address::ExtendedAddress;
use ::miniscript::{self, *};

use bitcoin::hashes::sha256;
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::util::amount::Amount;
use bitcoin::util::psbt::PartiallySignedTransaction;
use sapio_base::txindex::TxIndexError;
use sapio_base::txindex::{TxIndex, TxIndexLogger};
use sapio_base::Clause;
use sapio_ctv_emulator_trait::{CTVAvailable, CTVEmulator, EmulatorError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// Error types that can arise when constructing an Object
#[derive(Debug)]
pub enum ObjectError {
    /// The Error was due to Miniscript
    Miniscript(miniscript::policy::compiler::CompilerError),
    /// Unknown Script Type
    UnknownScriptType(bitcoin::Script),
    /// OpReturn Too Long
    OpReturnTooLong,
    /// The Error was for an unknown/unhandled reason
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
    /// a map of template hashes to the corresponding template, that in the
    /// policy are a CTV protected
    #[serde(
        rename = "template_hash_to_template_map",
        skip_serializing_if = "HashMap::is_empty",
        default
    )]
    pub ctv_to_tx: HashMap<sha256::Hash, Template>,
    /// a map of template hashes to the corresponding template, that in the
    /// policy are not necessarily CTV protected but we might want to know about
    /// anyways.
    #[serde(
        rename = "suggested_template_hash_to_template_map",
        skip_serializing_if = "HashMap::is_empty",
        default
    )]
    pub suggested_txs: HashMap<sha256::Hash, Template>,
    /// The Object's Policy -- if known
    #[serde(
        rename = "known_policy",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub policy: Option<Clause>,
    /// The Object's address, or a Script if no address is possible
    pub address: ExtendedAddress,
    /// The Object's descriptor -- if there is one known/available
    #[serde(
        rename = "known_descriptor",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub descriptor: Option<Descriptor<bitcoin::PublicKey>>,
    /// The amount_range safe to send this object
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
            address: address.into(),
            descriptor: None,
            amount_range: a.unwrap_or_else(|| {
                let mut a = AmountRange::new();
                a.update_range(Amount::min_value());
                a.update_range(Amount::from_sat(21_000_000 * 100_000_000));
                a
            }),
        }
    }

    /// Creates an object from a given script. The optional AmountRange argument determines the
    /// safe bounds the contract can receive, otherwise it is set to any.
    pub fn from_script(
        script: bitcoin::Script,
        a: Option<AmountRange>,
        net: bitcoin::Network,
    ) -> Result<Object, ObjectError> {
        bitcoin::Address::from_script(&script, net)
            .ok_or_else(|| ObjectError::UnknownScriptType(script.clone()))
            .map(|m| Object::from_address(m, a))
    }
    /// create an op_return of no more than 40 bytes
    pub fn from_op_return<'a, I: ?Sized>(data: &'a I) -> Result<Object, ObjectError>
    where
        &'a [u8]: From<&'a I>,
    {
        Ok(Object {
            ctv_to_tx: HashMap::new(),
            suggested_txs: HashMap::new(),
            policy: None,
            address: ExtendedAddress::make_op_return(data)?,
            descriptor: None,
            amount_range: AmountRange::new(),
        })
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
                &CTVAvailable,
            )
            .unwrap();
        (a.into_iter().map(|x| x.extract_tx()).collect(), b)
    }
    /// bind_psbt attaches and `Object` to a specific UTXO, returning a
    /// Vector of PSBTs and transaction metadata.
    ///
    /// `bind_psbt` accepts a CTVEmulator, a txindex, and a map of outputs to be
    /// bound to specific template hashes.
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
                    metadata_map_s2s,
                    outputs,
                    tx,
                    ..
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
                    "metadata" : metadata_map_s2s,
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
