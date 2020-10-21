use bitcoin::consensus::encode::*;
use bitcoin::util::amount::{Amount, CoinAmount};

use crate::contract::CompilationError;
use bitcoin::hashes::sha256;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::TryInto;
/// Metadata for outputs, arbitrary KV set.
pub type OutputMeta = HashMap<String, String>;

/// An Output is not a literal Bitcoin Output, but contains data needed to construct one, and
/// metadata for linking & ABI building
#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub struct Output {
    pub amount: CoinAmount,
    pub contract: crate::contract::Compiled,
    pub metadata: OutputMeta,
}
impl Output {
    /// Creates a new Output, forcing the compilation of the compilable object and defaulting
    /// metadata if not provided to blank.
    pub fn new<T: crate::contract::Compilable>(
        amount: CoinAmount,
        contract: T,
        metadata: Option<OutputMeta>,
    ) -> Result<Output, CompilationError> {
        Ok(Output {
            amount,
            contract: contract.compile()?,
            metadata: metadata.unwrap_or_else(HashMap::new),
        })
    }
}

/// TemplateBuilder can be used to interactively put together a transaction template before
/// finalizing into a Template.
pub struct TemplateBuilder {
    n_inputs: usize,
    sequences: Vec<u32>,
    outputs: Vec<Output>,
    version: i32,
    lock_time: u32,
    label: String,
}

impl TemplateBuilder {
    /// Creates a new transaction template with 1 input and no outputs.
    pub fn new() -> TemplateBuilder {
        TemplateBuilder {
            n_inputs: 1,
            sequences: vec![0],
            outputs: vec![],
            version: 2,
            lock_time: 0,
            label: String::new(),
        }
    }
    pub fn add_output(mut self, o: Output) -> Self {
        self.outputs.push(o);
        self
    }

    pub fn add_sequence(mut self, s: u32) -> Self {
        self.sequences.push(s);
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

    /// Creates a transaction from a TemplateBuilder.
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
                    script_pubkey: out.contract.descriptor.script_pubkey(),
                })
                .collect(),
        }
    }
}

impl From<TemplateBuilder> for Template {
    fn from(t: TemplateBuilder) -> Template {
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
impl From<TemplateBuilder> for Result<Template, CompilationError> {
    fn from(t: TemplateBuilder) -> Self {
        Ok(t.into())
    }
}

/// Any type which can generate a CTVHash. Allows some decoupling in the future if some types will
/// not be literal transactions.
trait CTVHash {
    fn get_ctv_hash(&self, input_index: u32) -> sha256::Hash;
    fn total_amount(&self) -> Amount;
}
impl CTVHash for bitcoin::Transaction {
    /// Uses BIP-119 Logic to compute a CTV Hash
    fn get_ctv_hash(&self, input_index: u32) -> sha256::Hash {
        let mut ctv_hash = sha256::Hash::engine();
        self.version.consensus_encode(&mut ctv_hash).unwrap();
        self.lock_time.consensus_encode(&mut ctv_hash).unwrap();
        (self.input.len() as u32)
            .consensus_encode(&mut ctv_hash)
            .unwrap();
        {
            let mut enc = sha256::Hash::engine();
            for seq in self.input.iter().map(|i| i.sequence) {
                seq.consensus_encode(&mut enc).unwrap();
            }
            sha256::Hash::from_engine(enc)
                .into_inner()
                .consensus_encode(&mut ctv_hash)
                .unwrap();
        }

        (self.output.len() as u32)
            .consensus_encode(&mut ctv_hash)
            .unwrap();

        {
            let mut enc = sha256::Hash::engine();
            for out in self.output.iter() {
                out.consensus_encode(&mut enc).unwrap();
            }
            sha256::Hash::from_engine(enc)
                .into_inner()
                .consensus_encode(&mut ctv_hash)
                .unwrap();
        }
        input_index.consensus_encode(&mut ctv_hash).unwrap();
        sha256::Hash::from_engine(ctv_hash)
    }

    fn total_amount(&self) -> Amount {
        Amount::from_sat(self.output.iter().fold(0, |a, b| a + b.value))
    }
}

/// Template holds the data needed to construct a Transaction for CTV Purposes, along with relevant
/// metadata
#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub struct Template {
    outputs: Vec<Output>,
    tx: bitcoin::Transaction,
    ctv: sha256::Hash,
    max: CoinAmount,
    label: String,
}

use bitcoin::hashes::Hash;
impl Template {
    pub fn hash(&self) -> sha256::Hash {
        self.ctv
    }
    pub fn new() -> TemplateBuilder {
        TemplateBuilder::new()
    }

    pub fn total_amount(&self) -> Amount {
        Amount::from_sat(0)
    }
}
