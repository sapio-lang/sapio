// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Interactive Transaction Template Builder
pub use super::{Output, OutputMeta};
use super::{Template, TemplateMetadata};
use crate::contract::{CompilationError, Context};
use bitcoin::util::amount::Amount;
use bitcoin::VarInt;
use miniscript::DescriptorTrait;
use sapio_base::effects::PathFragment;
use sapio_base::timelocks::*;
use sapio_base::CTVHash;
use std::convert::TryFrom;
use std::convert::TryInto;

/// Builder can be used to interactively put together a transaction template before
/// finalizing into a Template.
pub struct Builder {
    sequences: Vec<Option<AnyRelTimeLock>>,
    outputs: Vec<Output>,
    version: i32,
    lock_time: Option<AnyAbsTimeLock>,
    ctx: Context,
    fees: Amount,
    min_feerate: Option<Amount>,
    // Metadata Fields:
    metadata: TemplateMetadata,
}

impl Builder {
    /// Creates a new transaction template with 1 input and no outputs.
    pub fn new(ctx: Context) -> Builder {
        Builder {
            sequences: vec![None],
            outputs: vec![],
            version: 2,
            lock_time: None,
            metadata: TemplateMetadata::new(),
            fees: Amount::from_sat(0),
            min_feerate: None,
            ctx,
        }
    }

    /// get a read-only reference to the builder's context
    pub fn ctx(&self) -> &Context {
        &self.ctx
    }

    /// reduce the amount availble in the builder's context
    pub fn spend_amount(mut self, amount: Amount) -> Result<Self, CompilationError> {
        self.ctx = self.ctx.spend_amount(amount)?;
        Ok(self)
    }

    /// reduce the amount availble in the builder's context, and add to the fees
    pub fn add_fees(self, amount: Amount) -> Result<Self, CompilationError> {
        let mut c = self.spend_amount(amount)?;
        c.fees += amount;
        Ok(c)
    }

    /// Creates a new Output, forcing the compilation of the compilable object and defaulting
    /// metadata if not provided to blank.
    pub fn add_output(
        mut self,
        amount: Amount,
        contract: &dyn crate::contract::Compilable,
        metadata: Option<OutputMeta>,
    ) -> Result<Self, CompilationError> {
        let subctx = self
            .ctx
            .derive(PathFragment::Branch(self.outputs.len() as u64))?
            .with_amount(amount)?;
        let mut ret = self.spend_amount(amount)?;
        ret.outputs.push(Output {
            amount: amount,
            contract: contract.compile(subctx)?,
            metadata: metadata.unwrap_or_else(Default::default),
        });
        Ok(ret)
    }

    /// adds available funds to the builder's context object.
    /// TODO: Make guarantee there is some external input?
    pub fn add_amount(mut self, a: Amount) -> Self {
        self.ctx = self.ctx.add_amount(a);
        self
    }

    /// Adds another output. Follow with a call to
    /// set_sequence(-1, ...) to fill in the back.
    pub fn add_sequence(mut self) -> Self {
        self.sequences.push(None);
        self
    }
    /// set_sequence adds a height or time based relative lock time to the
    /// template. If a lock time is already set, it will check if it is of the
    /// same kind. Differing kinds will throw an error. Otherwise, it will merge
    /// by taking the max of the argument.
    ///
    /// Negative indexing allows us to work from the back element easily
    pub fn set_sequence(mut self, ii: isize, s: AnyRelTimeLock) -> Result<Self, CompilationError> {
        let i = if ii >= 0 {
            ii
        } else {
            self.sequences.len() as isize + ii
        } as usize;
        match self.sequences.get_mut(i).as_mut() {
            Some(Some(seq)) => match (*seq, s) {
                (a @ AnyRelTimeLock::RH(_), b @ AnyRelTimeLock::RH(_)) => {
                    *seq = std::cmp::max(a, b);
                }
                (a @ AnyRelTimeLock::RT(_), b @ AnyRelTimeLock::RT(_)) => {
                    *seq = std::cmp::max(a, b);
                }
                _ => return Err(CompilationError::IncompatibleSequence),
            },
            Some(x @ None) => {
                x.replace(s);
            }
            None => return Err(CompilationError::NoSuchSequence),
        };
        Ok(self)
    }
    /// set_lock_time adds a height or time based absolute lock time to the
    /// template. If a lock time is already set, it will check if it is of the
    /// same kind. Differing kinds will throw an error. Otherwise, it will merge
    /// by taking the max of the argument.
    pub fn set_lock_time(mut self, lt_in: AnyAbsTimeLock) -> Result<Self, CompilationError> {
        if let Some(lt) = self.lock_time.as_mut() {
            match (*lt, lt_in) {
                (a @ AnyAbsTimeLock::AH(_), b @ AnyAbsTimeLock::AH(_)) => {
                    *lt = std::cmp::max(a, b);
                }
                (a @ AnyAbsTimeLock::AT(_), b @ AnyAbsTimeLock::AT(_)) => {
                    *lt = std::cmp::max(a, b);
                }
                _ => return Err(CompilationError::IncompatibleSequence),
            }
        } else {
            self.lock_time = Some(lt_in);
        }
        Ok(self)
    }

    /// overwrite any existing label with the provided string,
    /// or set a label if none provided thus far.
    pub fn set_label(mut self, label: String) -> Self {
        self.metadata.label = Some(label);
        self
    }

    /// overwrite any existing color with the provided string,
    /// or set a color if none provided thus far.
    pub fn set_color(mut self, color: String) -> Self {
        self.metadata.color = Some(color);
        self
    }

    /// Creates a transaction from a Builder.
    /// Generally, should not be called directly.
    pub fn get_tx(&self) -> bitcoin::Transaction {
        let default_seq = RelTime::try_from(0).unwrap().into();
        let default_nlt = AbsHeight::try_from(0).unwrap().into();
        bitcoin::Transaction {
            version: self.version,
            lock_time: self.lock_time.unwrap_or(default_nlt).get(),
            input: self
                .sequences
                .iter()
                .map(|sequence| bitcoin::TxIn {
                    previous_output: Default::default(),
                    script_sig: Default::default(),
                    sequence: sequence.unwrap_or(default_seq).get(),
                    witness: vec![],
                })
                .collect(),
            output: self
                .outputs
                .iter()
                .map(|out| bitcoin::TxOut {
                    value: TryInto::<Amount>::try_into(out.amount).unwrap().as_sat(),
                    script_pubkey: out.contract.address.clone().into(),
                })
                .collect(),
        }
    }

    /// Sets the feerate if not set, and then sets the value to the min of the
    /// existing value or the new value.
    /// For example, s.set_min_feerate(100.into()).set_min_feerate(1000.into())
    /// results in feerate Some(100).
    ///
    /// During compilation, templates should be checked to ensure that at least
    /// that feerate is paid.
    pub fn set_min_feerate(mut self, a: Amount) -> Self {
        let v: &mut Amount = self.min_feerate.get_or_insert(a);
        *v = std::cmp::min(*v, a);
        self
    }

    /// more efficient that get_tx() to estimate a tx size, not including witness
    pub fn estimate_tx_size(&self) -> u64 {
        let mut input_weight: u64 = 0;
        let inputs_with_witnesses: u64 = self.sequences.len() as u64;
        let scale_factor = 1u64;
        for _seq in &self.sequences {
            input_weight += scale_factor
                * (32 + 4 + 4 + // outpoint (32+4) + nSequence
                VarInt(0u64).len() as u64 + 0);
            //if !input.witness.is_empty() {
            //    inputs_with_witnesses += 1;
            //    input_weight += VarInt(input.witness.len() as u64).len();
            //    for elem in &input.witness {
            //        input_weight += VarInt(elem.len() as u64).len() + elem.len();
            //    }
            //}
        }
        let mut output_size: u64 = 0;
        for output in &self.outputs {
            let spk = output
                .contract
                .descriptor
                .as_ref()
                .map(|d| d.script_pubkey().len() as u64);
            output_size += 8 + // value
                (VarInt(spk.unwrap_or(0)).len() as u64) +
                spk.unwrap_or(0);
        }
        let non_input_size : u64=
        // version:
        4 +
        // count varints:
        (VarInt(self.sequences.len() as u64).len() as u64 +
        VarInt(self.outputs.len() as u64).len() as u64)+
        output_size +
        // lock_time
        4;
        if inputs_with_witnesses == 0 {
            non_input_size * scale_factor + input_weight
        } else {
            non_input_size * scale_factor + input_weight + (self.sequences.len() as u64)
                - inputs_with_witnesses
                + 2
        }
    }
}
impl From<Builder> for Template {
    fn from(t: Builder) -> Template {
        let tx = t.get_tx();
        Template {
            outputs: t.outputs,
            ctv: tx.get_ctv_hash(0),
            ctv_index: 0,
            max: tx.total_amount() + t.fees,
            min_feerate_sats_vbyte: t.min_feerate,
            tx,
            metadata_map_s2s: t.metadata,
        }
    }
}

impl From<Builder> for crate::contract::TxTmplIt {
    fn from(t: Builder) -> Self {
        // t.into() // works too, but prefer the explicit form so we know what we get concretely
        Ok(Box::new(std::iter::once(Ok(t.into()))))
    }
}
