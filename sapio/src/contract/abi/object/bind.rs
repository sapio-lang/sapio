// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//!  binding Object to a specific UTXO
use super::descriptors::*;
use crate::contract::abi::continuation::ContinuationPoint;
pub use crate::contract::abi::studio::*;
use crate::contract::object::Object;
use crate::contract::object::ObjectError;
use crate::template::Template;
use crate::util::amountrange::AmountRange;
use crate::util::extended_address::ExtendedAddress;
use ::miniscript::{self, *};
use bitcoin::hashes::sha256;
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::util::amount::Amount;
use bitcoin::util::psbt::PartiallySignedTransaction;
use bitcoin::util::taproot::TaprootBuilder;
use bitcoin::util::taproot::TaprootSpendInfo;
use bitcoin::OutPoint;
use bitcoin::PublicKey;
use bitcoin::Script;
use bitcoin::XOnlyPublicKey;
use sapio_base::effects::EffectPath;
use sapio_base::effects::PathFragment;
use sapio_base::serialization_helpers::SArc;
use sapio_base::txindex::TxIndex;
use sapio_ctv_emulator_trait::CTVEmulator;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
impl Object {
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
    ) -> Result<Program, ObjectError> {
        let mut result = HashMap::<SArc<EffectPath>, SapioStudioObject>::new();
        // Could use a queue instead to do BFS linking, but order doesn't matter and stack is
        // faster.
        let mut stack = vec![(out_in, self)];
        let mut mock_out = OutPoint::default();
        mock_out.vout = 0;
        let secp = bitcoin::secp256k1::Secp256k1::new();
        while let Some((
            out,
            Object {
                root_path,
                continue_apis,
                descriptor,
                ctv_to_tx,
                suggested_txs,
                ..
            },
        )) = stack.pop()
        {
            result.insert(
                root_path.clone(),
                SapioStudioObject {
                    continue_apis: continue_apis.clone(),
                    txs: ctv_to_tx
                        .iter()
                        .chain(suggested_txs.iter())
                        .map(
                            |(
                                ctv_hash,
                                Template {
                                    metadata_map_s2s,
                                    outputs,
                                    tx,
                                    ..
                                },
                            )| {
                                let mut tx = tx.clone();
                                tx.input[0].previous_output = out;
                                for inp in tx.input[1..].iter_mut() {
                                    inp.previous_output = mock_out;
                                    mock_out.vout += 1;
                                }
                                if let Some(outputs) = output_map.get(ctv_hash) {
                                    for (i, inp) in tx.input.iter_mut().enumerate().skip(1) {
                                        if let Some(out) = outputs[i] {
                                            inp.previous_output = out;
                                        }
                                    }
                                }
                                let mut psbtx =
                                    PartiallySignedTransaction::from_unsigned_tx(tx.clone())
                                        .unwrap();
                                for (psbt_in, tx_in) in psbtx.inputs.iter_mut().zip(tx.input.iter())
                                {
                                    psbt_in.witness_utxo =
                                        blockdata.lookup_output(&tx_in.previous_output).ok();
                                }
                                // Missing other Witness Info.
                                match descriptor {
                                    Some(SupportedDescriptors::Pk(d)) => {
                                        psbtx.inputs[0].witness_script = Some(d.explicit_script()?);
                                    }
                                    Some(SupportedDescriptors::XOnly(Descriptor::Tr(t))) => {
                                        let mut builder = TaprootBuilder::new();
                                        let mut added = false;
                                        for (depth, ms) in t.iter_scripts() {
                                            added = true;
                                            let script = ms.encode();
                                            builder = builder.add_leaf(depth, script)?;
                                        }
                                        let info = if added {
                                            builder.finalize(&secp, t.internal_key().clone())?
                                        } else {
                                            TaprootSpendInfo::new_key_spend(
                                                &secp,
                                                t.internal_key().clone(),
                                                None,
                                            )
                                        };
                                        let inp = &mut psbtx.inputs[0];
                                        for item in info.as_script_map().keys() {
                                            let cb =
                                                info.control_block(item).expect("Must be present");
                                            inp.tap_scripts.insert(cb.clone(), item.clone());
                                        }
                                        inp.tap_merkle_root = info.merkle_root();
                                        inp.tap_internal_key = Some(info.internal_key());
                                    }
                                    _ => (),
                                }
                                psbtx = emulator.sign(psbtx)?;
                                let final_tx = psbtx.clone().extract_tx();
                                let txid = blockdata.add_tx(Arc::new(final_tx))?;
                                stack.reserve(outputs.len());
                                for (vout, v) in outputs.iter().enumerate() {
                                    let vout = vout as u32;
                                    stack.push((bitcoin::OutPoint { txid, vout }, &v.contract));
                                }
                                Ok(LinkedPSBT {
                                    psbt: psbtx,
                                    metadata: metadata_map_s2s.clone(),
                                    output_metadata: outputs
                                        .iter()
                                        .cloned()
                                        .map(|x| x.contract.metadata)
                                        .collect::<Vec<_>>(),
                                    added_output_metadata: outputs
                                        .iter()
                                        .cloned()
                                        .map(|x| x.added_metadata)
                                        .collect::<Vec<_>>(),
                                }
                                .into())
                            },
                        )
                        .collect::<Result<Vec<SapioStudioFormat>, ObjectError>>()?,
                },
            );
        }
        Ok(Program { program: result })
    }
}
