use bitcoin::util::amount::Amount;

use crate::contract::{CompilationError, Context};
use bitcoin::hashes::sha256;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::TryInto;

pub mod output;
pub use output::{Output, OutputMeta};
use sapio_base::CTVHash;

/// Builder can be used to interactively put together a transaction template before
/// finalizing into a Template.
pub struct Builder {
    sequences: Vec<u32>,
    outputs: Vec<Output>,
    version: i32,
    lock_time: u32,
    label: String,
    amount: Amount,
    ctx: Context,
}

impl Builder {
    /// Creates a new transaction template with 1 input and no outputs.
    pub fn new(ctx: Context) -> Builder {
        Builder {
            sequences: vec![0],
            outputs: vec![],
            version: 2,
            lock_time: 0,
            label: String::new(),
            amount: Amount::from_sat(0),
            ctx,
        }
    }
    pub fn spend_amount(mut self, amount: Amount) -> Result<Self, CompilationError> {
        self.ctx.spend_amount(amount)?;
        Ok(self)
    }

    /// Creates a new Output, forcing the compilation of the compilable object and defaulting
    /// metadata if not provided to blank.
    pub fn add_output<T: crate::contract::Compilable>(
        mut self,
        amount: Amount,
        contract: &T,
        metadata: Option<OutputMeta>,
    ) -> Result<Self, CompilationError> {
        self.outputs.push(Output {
            amount: amount,
            contract: contract.compile(&self.ctx.with_amount(amount)?)?,
            metadata: metadata.unwrap_or_else(HashMap::new),
        });
        self.spend_amount(amount)
    }

    // TODO: Make guarantee there is some external input?
    pub fn add_amount(mut self, a: Amount) -> Self {
        self.ctx.add_amount(a);
        self
    }

    pub fn add_sequence(mut self, s: u32) -> Self {
        self.sequences.push(s);
        self
    }
    pub fn set_sequence(mut self, i: usize, s: u32) -> Self {
        self.sequences[i] = s;
        self
    }
    /// TODO: Logic to validate that changes are not breaking
    pub fn set_lock_time(mut self, lt: u32) -> Self {
        self.lock_time = lt;
        self
    }

    pub fn set_label(mut self, label: String) -> Self {
        self.label = label;
        self
    }

    /// Creates a transaction from a Builder.
    /// Generally, should not be called directly.
    pub fn get_tx(&self) -> bitcoin::Transaction {
        bitcoin::Transaction {
            version: self.version,
            lock_time: self.lock_time,
            input: self
                .sequences
                .iter()
                .map(|sequence| bitcoin::TxIn {
                    previous_output: Default::default(),
                    script_sig: Default::default(),
                    sequence: *sequence,
                    witness: vec![],
                })
                .collect(),
            output: self
                .outputs
                .iter()
                .map(|out| bitcoin::TxOut {
                    value: TryInto::<Amount>::try_into(out.amount).unwrap().as_sat(),
                    script_pubkey: out.contract.address.script_pubkey(),
                })
                .collect(),
        }
    }
}

impl From<Builder> for Template {
    fn from(t: Builder) -> Template {
        let tx = t.get_tx();
        Template {
            outputs: t.outputs,
            ctv: tx.get_ctv_hash(0),
            max: tx.total_amount().into(),
            tx,
            label: t.label,
        }
    }
}
impl From<Builder> for Result<Template, CompilationError> {
    fn from(t: Builder) -> Self {
        Ok(t.into())
    }
}

impl From<Builder> for crate::contract::TxTmplIt {
    fn from(t: Builder) -> Self {
        // t.into() // works too, but prefer the explicit form so we know what we get concretely
        Ok(Box::new(std::iter::once(Result::<
            Template,
            CompilationError,
        >::from(t))))
    }
}

/// Template holds the data needed to construct a Transaction for CTV Purposes, along with relevant
/// metadata
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Template {
    pub outputs: Vec<Output>,
    pub tx: bitcoin::Transaction,
    pub ctv: sha256::Hash,
    #[serde(with = "bitcoin::util::amount::serde::as_sat")]
    #[schemars(with = "i64")]
    pub max: Amount,
    pub label: String,
}

impl Template {
    pub fn hash(&self) -> sha256::Hash {
        self.ctv
    }

    pub fn total_amount(&self) -> Amount {
        Amount::from_sat(0)
    }
}
