use bitcoin::consensus::encode::*;
use bitcoin::util::amount::Amount;

use bitcoin::hashes::sha256;
use std::collections::HashMap;
pub type OutputMeta = HashMap<String, String>;
#[derive(Clone)]
pub struct Output {
    pub amount: Amount,
    pub contract: crate::contract::Compiled,
    pub metadata: OutputMeta,
}
impl Output {
    pub fn new<T: crate::contract::Compilable>(
        amount: Amount,
        contract: T,
        metadata: Option<OutputMeta>,
    ) -> Output {
        Output {
            amount,
            contract: contract.compile(),
            metadata: metadata.unwrap_or_else(HashMap::new),
        }
    }
}

pub struct TemplateBuilder {
    n_inputs: usize,
    sequences: Vec<u32>,
    outputs: Vec<Output>,
    version: i32,
    lock_time: u32,
    label: String,
}

impl TemplateBuilder {
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

    pub fn set_lock_time(mut self, lt: u32) -> Self {
        self.lock_time = lt;
        self
    }

    pub fn set_label(mut self, label: String) -> Self {
        self.label = label;
        self
    }

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
                    value: out.amount.as_sat(),
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
            max: tx.total_amount(),
            tx,
            label: t.label,
        }
    }
}

trait CTVHash {
    fn get_ctv_hash(&self, input_index: u32) -> sha256::Hash;
    fn total_amount(&self) -> Amount;
}
impl CTVHash for bitcoin::Transaction {
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

#[derive(Clone)]
pub struct Template {
    outputs: Vec<Output>,
    tx: bitcoin::Transaction,
    ctv: sha256::Hash,
    max: Amount,
    label: String,
}

use bitcoin::hashes::Hash;
use bitcoin::hashes::HashEngine;
impl Template {
    pub fn hash(&self) -> sha256::Hash {
        self.ctv
    }
    pub fn relative_probability(&self) -> usize {
        1000
    }
    pub fn new() -> TemplateBuilder {
        TemplateBuilder::new()
    }

    pub fn total_amount(&self) -> Amount {
        Amount::from_sat(0)
    }
}
