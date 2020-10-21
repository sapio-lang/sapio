use crate::clause::Clause;
use crate::txn::Template;
use crate::txn::Template as TransactionTemplate;
use crate::util::amountrange::AmountRange;
use ::miniscript::*;
use bitcoin::hashes::sha256;
use bitcoin::util::amount::Amount;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
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
}

/// An Iterator which yields TransactionTemplates.
/// It is boxed to permit flexibility when returning.
pub type TxTmplIt<'a> = Box<dyn Iterator<Item = TransactionTemplate> + 'a>;

/// Compilable is a trait for anything which can be compiled
pub trait Compilable: private::ImplSeal {
    fn compile(&self) -> Compiled;
}

/// Implements a basic identity
impl Compilable for Compiled {
    fn compile(&self) -> Compiled {
        self.clone()
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
    fn compile(&self) -> Compiled {
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
        for (uses_ctv, guards, txtmpls) in then_fns.chain(finish_or_fns) {
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
                let hashes = Clause::Threshold(
                    1,
                    txtmpls
                        .map(|txtmpl| {
                            let h = txtmpl.hash();
                            let txtmpl = ctv_to_tx.entry(h).or_insert(txtmpl);
                            amount_range.update_range(txtmpl.total_amount());
                            Clause::TxTemplate(h)
                        })
                        .collect(),
                );
                option_guard = Some(match option_guard {
                    Some(guard) => Clause::And(vec![guard, hashes]),
                    None => hashes,
                });
            }
            option_guard.map(|guard| clause_accumulator.push(guard));
        }
        // TODO: Handle clause_accumulator.len() == 0
        let policy = Clause::Threshold(1, clause_accumulator);

        return Compiled {
            ctv_to_tx,
            // order flipped to borrow policy
            descriptor: Descriptor::Wsh(policy.compile().unwrap()),
            policy: Some(policy),
            amount_range,
        };
    }
}
