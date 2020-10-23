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

/// private::ImplSeal prevents anyone from implementing Compilable except by implementing Contract.
mod private {
    pub trait ImplSeal {}

    /// Allow Contract to implement Compile
    impl<T> ImplSeal for T where T: for<'a> super::Contract<'a> {}
    impl ImplSeal for super::Compiled {}
}

/// Compiled holds a contract's complete context required post-compilation
/// There is no guarantee that Compiled is properly constructed presently.
//TODO: Make type immutable and correct by construction...
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Compiled {
    pub ctv_to_tx: HashMap<sha256::Hash, Template>,
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
                ..
            },
        )) = stack.pop()
        {
            for (
                _ctv_hash,
                Template {
                    label, outputs, tx, ..
                },
            ) in ctv_to_tx
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

/// Compilable is a trait for anything which can be compiled
pub trait Compilable: private::ImplSeal {
    fn compile(&self) -> Result<Compiled, CompilationError>;
    fn from_json(s: serde_json::Value) -> Result<Compiled, CompilationError>
    where
        Self: for<'a> Deserialize<'a> + Compilable,
    {
        let t: Self =
            serde_json::from_value(s).map_err(|_| CompilationError::TerminateCompilation)?;
        let c = t.compile();
        c
    }
}

/// Implements a basic identity
impl Compilable for Compiled {
    fn compile(&self) -> Result<Compiled, CompilationError> {
        Ok(self.clone())
    }
}

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
    def! {then}
    def! { updatable<> }
    def! {finish}
}

use actions::Guard;
use std::marker::PhantomData;
struct GuardCache<T> {
    cache: HashMap<usize, Option<(Guard<T>, Option<Clause>)>>,
}
impl<T> GuardCache<T> {
    fn new() -> Self {
        GuardCache {
            cache: HashMap::new(),
        }
    }
    fn get(&mut self, t: &T, f: fn() -> Option<Guard<T>>) -> Option<Clause> {
        match self
            .cache
            .entry(f as usize)
            .or_insert_with(|| f().map(|v| (v, None)))
        {
            Some((Guard(g, true), e @ Some(..))) => e.clone(),
            Some((Guard(g, true), ref mut v @ None)) => {
                *v = Some(g(t));
                v.clone()
            }
            Some((Guard(g, false), None)) => Some(g(t)),

            Some((Guard(g, false), Some(..))) => std::panic!("Impossible"),
            None => None,
        }
    }
}

impl<T> Compilable for T
where
    T: for<'a> Contract<'a>,
{
    /// The main Compilation Logic for a Contract.
    /// TODO: Better Document Semantics
    fn compile(&self) -> Result<Compiled, CompilationError> {
        #[derive(PartialEq, Eq)]
        enum UsesCTV {
            Yes,
            No,
        }
        let mut guard_clauses = GuardCache::new();

        let finish_fns: Vec<_> = Self::FINISH_FNS
            .iter()
            .filter_map(|x| guard_clauses.get(self, *x))
            .collect();
        let mut clause_accumulator = if finish_fns.len() > 0 {
            vec![Clause::Threshold(1, finish_fns)]
        } else {
            vec![]
        };
        let mut ctv_to_tx = HashMap::new();

        let then_fns = Self::THEN_FNS
            .iter()
            .filter_map(|x| x())
            .map(|x| (UsesCTV::Yes, x.0, x.1(self)));
        let finish_or_fns = Self::FINISH_OR_FUNCS
            .iter()
            .filter_map(|x| x())
            .map(|x| (UsesCTV::No, x.guards(), x.fun()(self, Default::default())));

        let mut amount_range = AmountRange::new();
        for (uses_ctv, guards, r_txtmpls) in then_fns.chain(finish_or_fns) {
            let txtmpls = r_txtmpls?;
            // If no guards and not CTV, then nothing gets added (not interpreted as Trivial True)
            // If CTV and no guards, just CTV added.
            // If CTV and guards, CTV & guards added.
            let mut option_guard = guards
                .iter()
                .filter_map(|x| guard_clauses.get(self, *x))
                .fold(None, |option_guard, guard| {
                    Some(match option_guard {
                        None => guard,
                        Some(guards) => Clause::And(vec![guards, guard]),
                    })
                });
            if uses_ctv == UsesCTV::Yes {
                // TODO: Handle txtmpls.len() == 0
                //
                let tr: Result<Vec<_>, _> = txtmpls
                    .map(|r_txtmpl| {
                        let txtmpl = r_txtmpl?;
                        let h = txtmpl.hash();
                        let txtmpl = ctv_to_tx.entry(h).or_insert(txtmpl);
                        amount_range.update_range(txtmpl.total_amount());
                        Ok(Clause::TxTemplate(h))
                    })
                    .collect();
                let hashes = Clause::Threshold(1, tr?);
                option_guard = Some(match option_guard {
                    Some(guard) => Clause::And(vec![guard, hashes]),
                    None => hashes,
                });
            }
            option_guard.map(|guard| clause_accumulator.push(guard));
        }
        // TODO: Handle clause_accumulator.len() == 0
        let policy = Clause::Threshold(1, clause_accumulator);

        let descriptor = Descriptor::Wsh(policy.compile().unwrap());
        Ok(Compiled {
            ctv_to_tx,
            address: descriptor
                .address(bitcoin::Network::Bitcoin)
                .ok_or(CompilationError::TerminateCompilation)?,
            descriptor: Some(descriptor),
            // order flipped to borrow policy
            policy: Some(policy),
            amount_range,
        })
    }
}
