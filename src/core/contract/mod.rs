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
#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub struct Compiled {
    pub ctv_to_tx: HashMap<sha256::Hash, Template>,
    pub policy: Option<Clause>,
    pub descriptor: Descriptor<bitcoin::PublicKey>,
    pub amount_range: AmountRange,
}

impl Compiled {
    /// converts a descriptor and an optional AmountRange to a compiled object.
    /// This can be used for e.g. creating raw SegWit Scripts.
    pub fn from_descriptor(d: Descriptor<bitcoin::PublicKey>, a: Option<AmountRange>) -> Compiled {
        Compiled {
            ctv_to_tx: HashMap::new(),
            policy: None,
            descriptor: d,
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
                tx.input[0].witness = vec![descriptor.witness_script().into_bytes()];
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
    fn from_json(s: &str) -> Result<Compiled, CompilationError>
    where
        Self: for<'a> Deserialize<'a> + Compilable,
    {
        let t: Self =
            serde_json::from_str(s).map_err(|_| CompilationError::TerminateCompilation)?;
        t.compile()
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
        // Evaluate all Guards One Time and store in a map
        // TODO: Fixup after pointers made unstable
        let guard_clauses = {
            let mut guard_clauses: HashMap<usize, Clause> = HashMap::new();
            let guards2 = Self::FINISH_OR_FUNCS
                .iter()
                .filter_map(|x| x().map(|y| y.guards().iter()));
            let _guards3 = Self::FINISH_FNS.iter();
            for guards in Self::THEN_FNS
                .iter()
                .filter_map(|x| x().map(|y| y.0.iter()))
                .chain(guards2)
            {
                for guard in guards.filter_map(|x| x()) {
                    if guard.1 {
                        guard_clauses
                            .entry(guard.0 as usize)
                            .or_insert_with(|| guard.0(self));
                    }
                }
            }
            guard_clauses
        };

        let finish_fns: Vec<_> = Self::FINISH_FNS
            .iter()
            .filter_map(|x| x())
            .map(|x| {
                if x.1 {
                    guard_clauses[&(x.0 as usize)].clone()
                } else {
                    x.0(self)
                }
            })
            .collect();
        let mut clause_accumulator = vec![Clause::Threshold(1, finish_fns)];
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
                .filter_map(|x| x())
                .map(|guard| {
                    if guard.1 {
                        guard_clauses[&(guard.0 as usize)].clone()
                    } else {
                        guard.0(self)
                    }
                })
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

        Ok(Compiled {
            ctv_to_tx,
            // order flipped to borrow policy
            descriptor: Descriptor::Wsh(policy.compile().unwrap()),
            policy: Some(policy),
            amount_range,
        })
    }
}
