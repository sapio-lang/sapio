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
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::util::psbt::PartiallySignedTransaction;
use bitcoin::util::taproot::ControlBlock;
use bitcoin::util::taproot::TaprootBuilder;
use bitcoin::util::taproot::TaprootSpendInfo;
use bitcoin::{OutPoint, Transaction};
use miniscript::*;
use sapio_base::covenant::{Ctv, LoweringPlan};
use sapio_base::effects::{EffectPath, PathFragment};
use sapio_base::miniscript;
use sapio_base::serialization_helpers::SArc;
use sapio_base::txindex::{TxIndex, TxIndexError};
use sapio_base::Clause;
use sapio_ctv_emulator_trait::{sign_checked, CTVEmulator};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;

impl Object {
    /// Validate the graph against a signer's advertised covenant policies.
    /// Includes explicit wrapped guards as well as generated template checks.
    /// This does not prove signer availability or chain opcode enforcement.
    pub fn validate_for_emulator(&self, emulator: &dyn CTVEmulator) -> Result<(), ObjectError> {
        self.validate()?;
        self.validate_covenant_policies(|predicate| Ok(emulator.get_signer_for(predicate.0)?))
    }

    /// Validate reused compiled children against explicit public lowering data.
    /// This is a pure operation: it cannot resolve or contact a signer service.
    pub fn validate_for_lowering(&self, lowering: &LoweringPlan) -> Result<(), ObjectError> {
        self.validate()?;
        lowering.validate()?;
        self.validate_covenant_policies(|predicate| Ok(lowering.lower_ctv(predicate)?))
    }

    // The graph must pass structural validation before this traversal.
    fn validate_covenant_policies(
        &self,
        mut actual_policy: impl FnMut(Ctv) -> Result<Clause, ObjectError>,
    ) -> Result<(), ObjectError> {
        let mut pending = vec![self];
        while let Some(object) = pending.pop() {
            let requirements = &object.covenant_requirements;
            for predicate in &requirements.predicates {
                let expected = requirements.lowering.lower_ctv(*predicate)?;
                let actual = actual_policy(*predicate)?;
                if expected != actual {
                    return Err(ObjectError::CovenantPolicyMismatch {
                        path: object.root_path.clone(),
                        template: predicate.0,
                        expected: Box::new(expected),
                        actual: Box::new(actual),
                    });
                }
            }
            for template in object
                .ctv_to_tx
                .values()
                .chain(object.suggested_txs.values())
            {
                pending.extend(template.outputs.iter().map(|output| &output.contract));
            }
        }
        Ok(())
    }

    /// bind_psbt attaches and `Object` to a specific UTXO, returning a
    /// Vector of PSBTs and transaction metadata.
    ///
    /// `bind_psbt` accepts a CTVEmulator, a txindex, and a map of outputs to be
    /// bound to specific template hashes. Each supplied vector must contain
    /// one entry per transaction input. Entry zero must be `None`, since
    /// `out_in` and the parent transaction outputs determine contract inputs.
    /// Omitted mappings and `None` entries leave auxiliary inputs unresolved.
    ///
    /// The entire artifact, all mappings and all known funding are validated
    /// before signing. Only a matching `TxIndexError::UnknownTxid` leaves an
    /// explicit input unresolved; operational errors and invalid outputs fail.
    /// Known contract outputs must match the contract script. When every input
    /// is known, their total must cover the outputs and reserved fees.
    ///
    /// Every known previous transaction is included in the PSBT. This checks
    /// transaction identity, not confirmation or whether an output is unspent.
    /// All signer responses are checked before adding transactions to the index;
    /// an index write failure can still leave earlier writes in the index.
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
        self.validate_covenant_policies(|predicate| Ok(emulator.get_signer_for(predicate.0)?))?;
        // Prepare the complete graph before signing or inserting generated transactions.
        // Descendants use the actual generated parent, not a second index lookup.
        let mut prepared = Vec::new();
        let mut stack = vec![(
            out_in,
            self,
            self.root_path.clone(),
            lookup_funding(blockdata.as_ref(), out_in)?,
        )];
        let mut reserved: BTreeSet<_> = output_map
            .values()
            .flatten()
            .filter_map(|out| *out)
            .chain(std::iter::once(out_in))
            .collect();
        let mut mock_out = OutPoint {
            vout: 0,
            ..OutPoint::default()
        };
        let secp = bitcoin::secp256k1::Secp256k1::new();
        while let Some((out, object, bound_path, funding)) = stack.pop() {
            let invalid_funding = |outpoint, reason: String| ObjectError::InvalidFunding {
                path: object.root_path.clone(),
                outpoint,
                reason,
            };
            if let Some(previous) = &funding {
                if previous.output[out.vout as usize].script_pubkey
                    != bitcoin::Script::from(&object.address)
                {
                    return Err(invalid_funding(
                        out,
                        "output script does not match the contract".into(),
                    ));
                }
                let available = previous.output[out.vout as usize].value;
                if available < object.required_input_amount.as_sat() {
                    return Err(invalid_funding(
                        out,
                        format!(
                            "contract input provides {available} sat but requires {} sat",
                            object.required_input_amount.as_sat()
                        ),
                    ));
                }
            }
            let mut transactions = Vec::new();
            for (kind, hash, template) in object
                .ctv_to_tx
                .iter()
                .map(|(hash, template)| (PathFragment::Next, hash, template))
                .chain(
                    object
                        .suggested_txs
                        .iter()
                        .map(|(hash, template)| (PathFragment::Suggested, hash, template)),
                )
            {
                let mut tx = template.tx.clone();
                tx.input[0].previous_output = out;
                let mut seen = BTreeSet::from([out]);
                let mut prev_txs = vec![funding.clone()];
                for (i, input) in tx.input.iter_mut().enumerate().skip(1) {
                    let mapped = output_map.get(hash).and_then(|inputs| inputs[i]);
                    let previous = if let Some(mapped) = mapped {
                        input.previous_output = mapped;
                        lookup_funding(blockdata.as_ref(), mapped)?
                    } else {
                        // Placeholders remain explicitly unresolved even if an
                        // index happens to contain a transaction with this ID.
                        while reserved.contains(&mock_out) || mock_out == out {
                            mock_out.vout = mock_out.vout.checked_add(1).ok_or_else(|| {
                                invalid_funding(
                                    out,
                                    "exhausted unresolved input identifiers".into(),
                                )
                            })?;
                        }
                        input.previous_output = mock_out;
                        reserved.insert(mock_out);
                        None
                    };
                    if !seen.insert(input.previous_output) {
                        return Err(invalid_funding(
                            input.previous_output,
                            "transaction spends an input more than once".into(),
                        ));
                    }
                    prev_txs.push(previous);
                }
                let mut psbt = PartiallySignedTransaction::from_unsigned_tx(tx.clone())?;
                let mut amount = 0u64;
                let mut complete = true;
                for ((input, tx_in), previous) in
                    psbt.inputs.iter_mut().zip(&tx.input).zip(prev_txs)
                {
                    if let Some(previous) = previous {
                        let output = &previous.output[tx_in.previous_output.vout as usize];
                        amount = amount.checked_add(output.value).ok_or_else(|| {
                            invalid_funding(
                                tx_in.previous_output,
                                "input amount sum overflows".into(),
                            )
                        })?;
                        if output.script_pubkey.is_witness_program() {
                            input.witness_utxo = Some(output.clone());
                        }
                        // The transaction lets downstream signers authenticate
                        // amounts and scripts against the committed outpoint.
                        input.non_witness_utxo = Some((*previous).clone());
                    } else {
                        complete = false;
                    }
                }
                if complete && amount < template.max.as_sat() {
                    return Err(invalid_funding(
                        out,
                        format!(
                        "inputs provide {amount} sat but outputs and reserved fees require {} sat",
                        template.max.as_sat()
                    ),
                    ));
                }
                match &object.descriptor {
                    Some(SupportedDescriptors::Pk(d)) => {
                        psbt.inputs[0].witness_script = Some(d.explicit_script()?);
                    }
                    Some(SupportedDescriptors::XOnly(Descriptor::Tr(t))) => {
                        let mut builder = TaprootBuilder::new();
                        let mut added = false;
                        for (depth, ms) in t.iter_scripts() {
                            added = true;
                            builder = builder.add_leaf(depth, ms.encode())?;
                        }
                        let info = if added {
                            builder.finalize(&secp, *t.internal_key())?
                        } else {
                            TaprootSpendInfo::new_key_spend(&secp, *t.internal_key(), None)
                        };
                        let input = &mut psbt.inputs[0];
                        for item in info.as_script_map().keys() {
                            let cb = info.control_block(item).expect("Must be present");
                            input.tap_scripts.insert(cb, item.clone());
                        }
                        input.tap_merkle_root = info.merkle_root();
                        input.tap_internal_key = Some(info.internal_key());
                    }
                    Some(SupportedDescriptors::Taproot(tree)) => {
                        let info = tree.spend_info();
                        let input = &mut psbt.inputs[0];
                        for ((script, version), branches) in info.as_script_map() {
                            // Preserve every proof when the same script occurs
                            // at different positions in the explicit tree.
                            for branch in branches {
                                let control = ControlBlock {
                                    leaf_version: *version,
                                    output_key_parity: info.output_key_parity(),
                                    internal_key: info.internal_key(),
                                    merkle_branch: branch.clone(),
                                };
                                input
                                    .tap_scripts
                                    .insert(control, (script.clone(), *version));
                            }
                        }
                        input.tap_merkle_root = info.merkle_root();
                        input.tap_internal_key = Some(info.internal_key());
                    }
                    _ => (),
                }
                let txid = tx.txid();
                let parent = Arc::new(tx);
                // Source paths can be reused. An occurrence is identified by
                // its parent transition and output, independently of metadata.
                let transition_path = EffectPath::push(
                    Some(EffectPath::push(Some(bound_path.0.clone()), kind)),
                    PathFragment::Named(SArc(Arc::new(hash.to_string()))),
                );
                for (vout, output) in template.outputs.iter().enumerate() {
                    stack.push((
                        OutPoint::new(txid, vout as u32),
                        &output.contract,
                        SArc(EffectPath::push(
                            Some(transition_path.clone()),
                            PathFragment::Branch(vout as u64),
                        )),
                        Some(parent.clone()),
                    ));
                }
                transactions.push((template, psbt));
            }
            prepared.push((out, object, bound_path, transactions));
        }
        // A late invalid signer response must not leave an indexed prefix.
        for (_, _, _, transactions) in &mut prepared {
            for (_, psbt) in transactions {
                *psbt = sign_checked(emulator, psbt.clone())?;
            }
        }
        let mut result = BTreeMap::<SArc<EffectPath>, SapioStudioObject>::new();
        for (out, object, bound_path, transactions) in prepared {
            let mut txs = Vec::with_capacity(transactions.len());
            for (template, psbt) in transactions {
                let tx = Arc::new(psbt.clone().extract_tx());
                let expected = tx.txid();
                let actual = blockdata.add_tx(tx)?;
                if actual != expected {
                    return Err(TxIndexError::TxidMismatch { expected, actual }.into());
                }
                txs.push(
                    LinkedPSBT {
                        psbt,
                        metadata: template.metadata_map_s2s.clone(),
                        output_metadata: template
                            .outputs
                            .iter()
                            .map(|x| x.contract.metadata.clone())
                            .collect(),
                        added_output_metadata: template
                            .outputs
                            .iter()
                            .map(|x| x.added_metadata.clone())
                            .collect(),
                    }
                    .into(),
                );
            }
            result.insert(
                bound_path,
                SapioStudioObject {
                    source_path: Some(object.root_path.clone()),
                    metadata: object.metadata.clone(),
                    out,
                    continue_apis: object.continue_apis.clone(),
                    txs,
                },
            );
        }
        Ok(Program { program: result })
    }
}

// Use lookup_tx directly: a custom lookup_output implementation must not be
// able to bypass transaction identity or output-bound checks at this boundary.
fn lookup_funding(
    index: &dyn TxIndex,
    out: OutPoint,
) -> Result<Option<Arc<Transaction>>, TxIndexError> {
    let tx = match index.lookup_tx(&out.txid) {
        Ok(tx) => tx,
        Err(TxIndexError::UnknownTxid(txid)) if txid == out.txid => return Ok(None),
        Err(error) => return Err(error),
    };
    let actual = tx.txid();
    if actual != out.txid {
        return Err(TxIndexError::TxidMismatch {
            expected: out.txid,
            actual,
        });
    }
    if out.vout as usize >= tx.output.len() {
        return Err(TxIndexError::IndexTooHigh(out.vout));
    }
    Ok(Some(tx))
}
