// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//!  binding Object to a specific UTXO
use super::descriptors::*;
pub use crate::contract::abi::studio::*;
use crate::contract::object::Object;
use crate::contract::object::ObjectError;
use crate::template::Template;
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::util::psbt::PartiallySignedTransaction;
use bitcoin::util::taproot::TaprootBuilder;
use bitcoin::util::taproot::TaprootSpendInfo;
use bitcoin::OutPoint;
use miniscript::*;
use sapio_base::effects::EffectPath;
use sapio_base::miniscript;
use sapio_base::serialization_helpers::SArc;
use sapio_base::txindex::TxIndex;
use sapio_ctv_emulator_trait::CTVEmulator;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;

impl Object {
    /// bind_psbt attaches and `Object` to a specific UTXO, returning a
    /// Vector of PSBTs and transaction metadata.
    ///
    /// `bind_psbt` accepts a CTVEmulator, a txindex, and a map of outputs to be
    /// bound to specific template hashes. Each supplied vector must contain
    /// one entry per transaction input. Entry zero must be `None`, since
    /// `out_in` and the parent transaction outputs determine contract inputs.
    /// Omitted mappings and `None` entries leave auxiliary inputs unresolved.
    ///
    /// The entire artifact and all mappings are validated before signing or
    /// adding transactions to the index.
    pub fn bind_psbt(
        &self,
        out_in: bitcoin::OutPoint,
        output_map: BTreeMap<Sha256, Vec<Option<bitcoin::OutPoint>>>,
        blockdata: Rc<dyn TxIndex>,
        emulator: &dyn CTVEmulator,
    ) -> Result<Program, ObjectError> {
        self.validate()?;
        if !output_map.is_empty() {
            let mut remaining: BTreeSet<_> = output_map.keys().collect();
            let mut pending = vec![self];
            while let Some(object) = pending.pop() {
                for (hash, template) in object.ctv_to_tx.iter().chain(&object.suggested_txs) {
                    if let Some(inputs) = output_map.get(hash) {
                        let reason = if inputs.len() != template.tx.input.len() {
                            Some("expected one entry per transaction input")
                        } else if inputs[0].is_some() {
                            Some(
                                "entry zero must be None; the contract input is bound by the graph",
                            )
                        } else {
                            None
                        };
                        if let Some(reason) = reason {
                            return Err(ObjectError::InvalidInputMapping {
                                template: *hash,
                                reason,
                            });
                        }
                        remaining.remove(hash);
                    }
                    pending.extend(template.outputs.iter().map(|output| &output.contract));
                }
            }
            if let Some(hash) = remaining.first() {
                return Err(ObjectError::InvalidInputMapping {
                    template: **hash,
                    reason: "template does not exist in the artifact",
                });
            }
        }
        let mut result = BTreeMap::<SArc<EffectPath>, SapioStudioObject>::new();
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
                metadata,
                ..
            },
        )) = stack.pop()
        {
            result.insert(
                root_path.clone(),
                SapioStudioObject {
                    metadata: metadata.clone(),
                    out,
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
                                    PartiallySignedTransaction::from_unsigned_tx(tx.clone())?;
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
                                            builder.finalize(&secp, *t.internal_key())?
                                        } else {
                                            TaprootSpendInfo::new_key_spend(
                                                &secp,
                                                *t.internal_key(),
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
