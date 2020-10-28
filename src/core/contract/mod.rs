use crate::clause::Clause;
use crate::txn::Template;
use crate::txn::Template as TransactionTemplate;
use crate::util::amountrange::AmountRange;
use ::miniscript::*;
use bitcoin::hashes::sha256;
use bitcoin::util::amount::Amount;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

#[macro_use]
pub mod macros;
pub mod actions;
pub mod compiler;

pub use compiler::Compilable;

/// Compiled holds a contract's complete context required post-compilation
/// There is no guarantee that Compiled is properly constructed presently.
//TODO: Make type immutable and correct by construction...
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Compiled {
    pub ctv_to_tx: HashMap<sha256::Hash, Template>,
    pub suggested_txs: HashMap<sha256::Hash, Template>,
    pub policy: Option<Clause>,
    pub address: bitcoin::Address,
    pub descriptor: Option<Descriptor<bitcoin::PublicKey>>,
    pub amount_range: AmountRange,
}

impl Compiled {
    /// converts a descriptor and an optional AmountRange to a compiled object.
    /// This can be used for e.g. creating raw SegWit Scripts.
    pub fn from_descriptor(d: Descriptor<bitcoin::PublicKey>, a: Option<AmountRange>) -> Compiled {
        Compiled {
            ctv_to_tx: HashMap::new(),
            suggested_txs: HashMap::new(),
            policy: None,
            address: d.address(bitcoin::Network::Bitcoin).unwrap(),
            descriptor: Some(d),
            amount_range: a.unwrap_or_else(|| {
                let mut a = AmountRange::new();
                a.update_range(Amount::min_value());
                a.update_range(Amount::max_value());
                a
            }),
        }
    }

    pub fn from_address(address: bitcoin::Address, a: Option<AmountRange>) -> Compiled {
        Compiled {
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

    pub fn bind(
        &self,
        out_in: bitcoin::OutPoint,
    ) -> (Vec<bitcoin::Transaction>, Vec<serde_json::Value>) {
        let mut txns = vec![];
        let mut metadata_out = vec![];
        // Could use a queue instead to do BFS linking, but order doesn't matter and stack is
        // faster.
        let mut stack = vec![(out_in, self)];

        while let Some((
            out,
            Compiled {
                descriptor,
                ctv_to_tx,
                suggested_txs,
                ..
            },
        )) = stack.pop()
        {
            for (
                _ctv_hash,
                Template {
                    label, outputs, tx, ..
                },
            ) in ctv_to_tx.iter().chain(suggested_txs.iter())
            {
                let mut tx = tx.clone();
                tx.input[0].previous_output = out;
                // Missing other Witness Info.
                if let Some(d) = descriptor {
                    tx.input[0].witness = vec![d.witness_script().into_bytes()];
                }
                let txid = tx.txid();
                txns.push(tx);
                metadata_out.push(json!({
                    "color" : "green",
                    "label" : label,
                    "utxo_metadata" : outputs.iter().map(|x| &x.metadata).collect::<Vec<_>>()
                }));
                for (vout, v) in outputs.iter().enumerate() {
                    let vout = vout as u32;
                    stack.push((bitcoin::OutPoint { txid, vout }, &v.contract));
                }
            }
        }
        (txns, metadata_out)
    }
}

use std::error::Error;
use std::fmt;
#[derive(Debug)]
pub enum CompilationError {
    TerminateCompilation,
}
impl fmt::Display for CompilationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Error for CompilationError {}
/// An Iterator which yields TransactionTemplates.
/// It is boxed to permit flexibility when returning.
pub type TxTmplIt<'a> = Result<
    Box<dyn Iterator<Item = Result<TransactionTemplate, CompilationError>> + 'a>,
    CompilationError,
>;

/// A catch-all type for any function that is a FinishOrFunc.
/// Unfortunately, because type signatures must all match, it's not
/// possible to have differing types across FinishOrFunc for a contract at compile time.
/// Use an enum if need be.
///
/// TODO: use associated-type defaults here!
pub trait Contract<'a>
where
    Self: Sized + 'a,
    Option<&'a Self::StatefulArguments>: Default,
{
    //! Main Contract Trait
    declare! {then}
    declare! { updatable<> }
    declare! {finish}
}

struct DynamicContractRef<'a, T, S> {
    then: &'a [fn() -> Option<actions::ThenFunc<'a, S>>],
    finish_or: &'a [fn() -> Option<actions::FinishOrFunc<'a, S, T>>],
    finish: &'a [fn() -> Option<actions::Guard<S>>],
    data: &'a S,
}
impl<'a, T, S> AnyContract<'a> for DynamicContractRef<'a, T, S> {
    type StatefulArguments = T;
    type Ref = S;
    fn then_fns(&self) -> &'a [fn() -> Option<actions::ThenFunc<'a, S>>] {
        self.then
    }
    fn finish_or_fns(
        &self,
    ) -> &'a [fn() -> Option<actions::FinishOrFunc<'a, S, Self::StatefulArguments>>] {
        self.finish_or
    }
    fn finish_fns(&self) -> &'a [fn() -> Option<actions::Guard<S>>] {
        self.finish
    }
    fn get_inner_ref(&self) -> &Self::Ref {
        self.data
    }
}

pub trait AnyContract<'a>
where
    Self: Sized + 'a,
{
    type StatefulArguments;
    type Ref;
    fn then_fns(&self) -> &'a [fn() -> Option<actions::ThenFunc<'a, Self::Ref>>];
    fn finish_or_fns(
        &self,
    ) -> &'a [fn() -> Option<actions::FinishOrFunc<'a, Self::Ref, Self::StatefulArguments>>];
    fn finish_fns(&self) -> &'a [fn() -> Option<actions::Guard<Self::Ref>>];
    fn get_inner_ref(&self) -> &Self::Ref;
}

impl<'a, C, T> AnyContract<'a> for C
where
    C: Contract<'a, StatefulArguments = T> + Sized,
{
    type StatefulArguments = T;
    type Ref = Self;
    fn then_fns(&self) -> &'a [fn() -> Option<actions::ThenFunc<'a, Self::Ref>>] {
        Self::THEN_FNS
    }
    fn finish_or_fns(
        &self,
    ) -> &'a [fn() -> Option<actions::FinishOrFunc<'a, Self::Ref, Self::StatefulArguments>>] {
        Self::FINISH_OR_FUNCS
    }
    fn finish_fns(&self) -> &'a [fn() -> Option<actions::Guard<Self::Ref>>] {
        Self::FINISH_FNS
    }
    fn get_inner_ref(&self) -> &Self::Ref {
        self
    }
}
