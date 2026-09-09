// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Interactive Transaction Template Builder
use super::input::InputMetadata;
pub use super::{Output, OutputMeta};
use super::{Template, TemplateMetadata};
use crate::contract::{CompilationError, Context};
use crate::util::extended_address::ExtendedAddress;
use bitcoin::util::amount::Amount;
use bitcoin::Witness;
use bitcoin::{Script, VarInt};
use sapio_base::effects::PathFragment;
use sapio_base::policy::ScriptPolicy;
use sapio_base::simp::SIMPAttachableAt;
use sapio_base::simp::TemplateInputLT;
use sapio_base::simp::TemplateLT;
use sapio_base::timelocks::*;
use sapio_base::CTVHash;
use std::convert::TryFrom;
use std::marker::PhantomData;

/// State Type Tag for NotAddingFees
pub struct NotAddingFees;
/// State Type Tag for AddingFees (forces fees to be added last)
pub struct AddingFees;
/// Builder can be used to interactively put together a transaction template before
/// finalizing into a Template.
///
/// Funds are debited only by adding outputs or fees; callers cannot discard an
/// ordinal prefix without recording where those sats go:
///
/// ```compile_fail
/// use bitcoin::Amount;
/// use sapio::template::Builder;
/// fn discard(builder: Builder) {
///     let _ = builder.spend_amount(Amount::from_sat(1));
/// }
/// ```
///
/// Adding fees closes output construction, including when the fee is zero:
///
/// ```compile_fail
/// use bitcoin::Amount;
/// use sapio::contract::Compiled;
/// use sapio::template::Builder;
/// fn output_after_fees(builder: Builder, child: &Compiled) {
///     let builder = builder.add_fees(Amount::ZERO).unwrap();
///     let _ = builder.add_output(Amount::from_sat(1), child, None);
/// }
/// ```
pub struct BuilderState<State> {
    guards: Vec<ScriptPolicy>,
    // TODO: Should be Comitted/Uncomitted if not CTV
    sequences: Vec<Option<AnyRelTimeLock>>,
    outputs: Vec<Output>,
    inputs: Vec<InputMetadata>,
    version: i32,
    lock_time: Option<AnyAbsTimeLock>,
    ctx: Context,
    initial_funding: Amount,
    external_funding: Amount,
    fees: Amount,
    min_feerate: Option<Amount>,
    // Metadata Fields:
    metadata: TemplateMetadata,
    _pd: PhantomData<State>,
}

///  Start state of a Builder
pub type Builder = BuilderState<NotAddingFees>;

impl BuilderState<NotAddingFees> {
    /// Creates a new transaction template with 1 input and no outputs.
    pub fn new(ctx: Context) -> BuilderState<NotAddingFees> {
        let initial_funding = ctx.funds();
        Self {
            guards: Vec::new(),
            sequences: vec![None],
            inputs: vec![InputMetadata::default()],
            outputs: vec![],
            version: 2,
            lock_time: None,
            metadata: TemplateMetadata::new(),
            fees: Amount::from_sat(0),
            min_feerate: None,
            ctx,
            initial_funding,
            external_funding: Amount::ZERO,
            _pd: Default::default(),
        }
    }

    /// get a read-only reference to the builder's context
    pub fn ctx(&self) -> &Context {
        &self.ctx
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
        let at = subctx.path().as_ref().clone();
        let contract = contract.compile(subctx)?;
        if amount < contract.required_input_amount {
            return Err(CompilationError::UnderfundedOutput {
                at,
                available: amount,
                required: contract.required_input_amount,
            });
        }
        let mut ret = self.spend_amount(amount)?;
        ret.outputs.push(Output {
            amount,
            contract,
            added_metadata: metadata.unwrap_or_default(),
        });
        Ok(ret)
    }

    /// Add funds contributed by auxiliary inputs. Add their sequences first.
    /// These funds contribute to the transaction total, not input zero's
    /// required amount. Binding verifies the actual funding inputs.
    pub fn add_amount(mut self, a: Amount) -> Result<Self, CompilationError> {
        if a != Amount::ZERO && self.sequences.len() == 1 {
            return Err(CompilationError::TerminateWith(
                "External funding requires an auxiliary input".into(),
            ));
        }
        let external = self
            .external_funding
            .checked_add(a)
            .ok_or(CompilationError::OutOfFunds)?;
        // Check all inputs, including money already allocated to outputs.
        // Checking only the remaining context would permit aggregate overflow.
        self.initial_funding
            .checked_add(external)
            .ok_or(CompilationError::OutOfFunds)?;
        self.ctx = self.ctx.add_amount(a)?;
        self.external_funding = external;
        Ok(self)
    }

    /// Adds another output. Follow with a call to
    /// set_sequence(-1, ...) to fill in the back.
    pub fn add_sequence(mut self) -> Self {
        self.sequences.push(None);
        self.inputs.push(Default::default());
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

    /// attempts to add a SIMP to the output meta.
    ///
    /// Returns [`SIMPError::AlreadyDefined`] if one was previously set.
    pub fn add_simp_for_input<S: SIMPAttachableAt<TemplateInputLT>>(
        mut self,
        ii: isize,
        s: S,
    ) -> Result<Self, CompilationError> {
        let i = if ii >= 0 {
            ii
        } else {
            self.sequences.len() as isize + ii
        } as usize;
        match self.inputs.get_mut(i) {
            Some(r) => {
                r.add_simp_inplace(s)?;
                Ok(self)
            }
            None => Err(CompilationError::NoSuchSequence),
        }
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
    /// set an extra metadata value
    pub fn set_extra_meta<I, J>(mut self, i: I, j: J) -> Result<Self, CompilationError>
    where
        I: Into<String>,
        J: Into<serde_json::Value>,
    {
        self.metadata = self.metadata.set_extra(i, j)?;
        Ok(self)
    }

    /// attempts to add a SIMP to the output meta.
    ///
    /// Returns [`SIMPError::AlreadyDefined`] if one was previously set.
    pub fn add_simp<S: SIMPAttachableAt<TemplateLT>>(
        mut self,
        s: S,
    ) -> Result<Self, CompilationError> {
        self.metadata = self.metadata.add_simp(s)?;
        Ok(self)
    }

    /// adds an additional precondition on this template
    /// which ends up being computed as:
    /// And(And(Top Guard, And(add_guard(1),..., add_guard(n))), CTV)
    /// n.b. not to be used with a continuation!
    pub fn add_guard(mut self, guard: impl Into<ScriptPolicy>) -> Self {
        self.guards.push(guard.into());
        self
    }

    /// Compile another policy language as an additional template precondition.
    /// Backend errors propagate before the template can be returned.
    pub fn add_policy(
        self,
        policy: &impl sapio_base::policy::PolicyCompiler,
    ) -> Result<Self, CompilationError> {
        Ok(self.add_guard(policy.compile_policy()?))
    }
}

impl<T> BuilderState<T> {
    /// Debit funds only while recording an output or explicit fee.
    fn spend_amount(mut self, amount: Amount) -> Result<Self, CompilationError> {
        self.ctx = self.ctx.spend_amount(amount)?;
        Ok(self)
    }
    /// reduce the amount availble in the builder's context, and add to the fees
    pub fn add_fees(self, amount: Amount) -> Result<BuilderState<AddingFees>, CompilationError> {
        let s = BuilderState {
            _pd: Default::default(),
            guards: self.guards,
            sequences: self.sequences,
            outputs: self.outputs,
            inputs: self.inputs,
            version: self.version,
            lock_time: self.lock_time,
            ctx: self.ctx,
            initial_funding: self.initial_funding,
            external_funding: self.external_funding,
            fees: self.fees,
            min_feerate: self.min_feerate,
            metadata: self.metadata,
        };
        let mut c = s.spend_amount(amount)?;
        c.fees = c
            .fees
            .checked_add(amount)
            .ok_or(CompilationError::OutOfFunds)?;
        Ok(c)
    }

    /// Creates a transaction from a Builder.
    /// Generally, should not be called directly.
    pub fn get_tx(&self) -> bitcoin::Transaction {
        let default_seq = RelTime::try_from(0).unwrap().into();
        let default_nlt = AbsHeight::try_from(0).unwrap().into();
        let input = self
            .sequences
            .iter()
            .map(|sequence| bitcoin::TxIn {
                previous_output: Default::default(),
                script_sig: Default::default(),
                sequence: sequence.unwrap_or(default_seq).get(),
                witness: Witness::new(),
            })
            .collect();
        let output = self
            .outputs
            .iter()
            .map(|out| {
                let value = out.amount.as_sat();

                let script_pubkey: Script = From::<&ExtendedAddress>::from(&out.contract.address);
                bitcoin::TxOut {
                    value,
                    script_pubkey,
                }
            })
            .collect();
        let t = bitcoin::Transaction {
            version: self.version,
            lock_time: self.lock_time.unwrap_or(default_nlt).get(),
            input,
            output,
        };
        t
    }

    /// Requires at least this many satoshis per virtual byte, retaining the
    /// strongest requirement when called more than once.
    ///
    /// Fees must be explicitly reserved with [`Self::add_fees`]. Compilation
    /// checks a conservative satisfaction estimate for single-input templates.
    /// Multiple-input templates are rejected because their other satisfactions
    /// are not known before binding.
    pub fn set_min_feerate(mut self, a: Amount) -> Self {
        let v: &mut Amount = self.min_feerate.get_or_insert(a);
        *v = std::cmp::max(*v, a);
        self
    }

    /// Exact serialized size of the current unsigned transaction, in bytes.
    ///
    /// Includes every output's actual script and CompactSize length prefixes.
    /// Inputs have empty scriptSigs and witnesses, so this excludes the SegWit
    /// marker/flag and all future satisfaction data. It is not a fee guarantee
    /// for the signed transaction.
    pub fn unsigned_tx_size(&self) -> u64 {
        let inputs = self.sequences.len() as u64;
        let outputs = self.outputs.len() as u64;
        let output_bytes: u64 = self
            .outputs
            .iter()
            .map(|output| {
                let script = Script::from(&output.contract.address);
                let size = script.len() as u64;
                8 + VarInt(size).len() as u64 + size
            })
            .sum();
        // Each unsigned input has an outpoint, an empty scriptSig prefix and
        // a sequence. Neither its placeholder outpoint nor its lock changes
        // its encoded size.
        4 + VarInt(inputs).len() as u64
            + inputs * 41
            + VarInt(outputs).len() as u64
            + output_bytes
            + 4
    }

    /// Size after appending an output with this script, without allocating it.
    ///
    /// The output value occupies eight bytes regardless of its eventual amount.
    /// Includes a wider output-count CompactSize prefix when necessary. Like
    /// [`Self::unsigned_tx_size`], this excludes future input satisfactions.
    pub fn unsigned_tx_size_with_output(&self, script_pubkey: &Script) -> u64 {
        let count = self.outputs.len() as u64;
        let size = script_pubkey.len() as u64;
        self.unsigned_tx_size() + VarInt(count + 1).len() as u64 - VarInt(count).len() as u64
            + 8
            + VarInt(size).len() as u64
            + size
    }
}

impl<T> From<BuilderState<T>> for Template {
    fn from(t: BuilderState<T>) -> Template {
        let tx = t.get_tx();
        // Private builder fields and checked allocation guarantee that all
        // outputs and fees fit within the checked aggregate input budget.
        let max = tx.total_amount() + t.fees;
        let required_input_amount = max.checked_sub(t.external_funding).unwrap_or(Amount::ZERO);
        Template {
            guards: t.guards,
            outputs: t.outputs,
            inputs: t.inputs,
            ctv: tx.get_ctv_hash(0),
            ctv_index: 0,
            max,
            required_input_amount,
            min_feerate_sats_vbyte: t.min_feerate,
            tx,
            metadata_map_s2s: t.metadata,
        }
    }
}

impl<T> From<BuilderState<T>> for crate::contract::TxTmplIt {
    fn from(t: BuilderState<T>) -> Self {
        // t.into() // works too, but prefer the explicit form so we know what we get concretely
        Ok(Box::new(std::iter::once(Ok(t.into()))))
    }
}
