use super::AnyContract;
use super::CompilationError;
use super::Compiled;
use crate::util::amountrange::AmountRange;
use serde::Deserialize;

use super::actions::Guard;
use crate::clause::Clause;
use ::miniscript::*;
use std::collections::HashMap;

enum CacheEntry<T> {
    Nothing,
    Cached(Clause),
    Fresh(fn(&T) -> Clause),
}

struct GuardCache<T> {
    cache: HashMap<usize, CacheEntry<T>>,
}
impl<T> GuardCache<T> {
    fn new() -> Self {
        GuardCache {
            cache: HashMap::new(),
        }
    }
    fn create_entry(g: Guard<T>, t: &T) -> CacheEntry<T> {
        match g {
            Guard(f, true) => CacheEntry::Cached(f(t)),
            Guard(f, false) => CacheEntry::Fresh(f),
        }
    }
    fn get(&mut self, t: &T, f: fn() -> Option<Guard<T>>) -> Option<Clause> {
        match self.cache.entry(f as usize).or_insert_with(|| {
            f().map(|v| Self::create_entry(v, t))
                .unwrap_or(CacheEntry::Nothing)
        }) {
            CacheEntry::Nothing => None,
            CacheEntry::Cached(s) => Some(s.clone()),
            CacheEntry::Fresh(f) => Some(f(t)),
        }
    }
}

/// private::ImplSeal prevents anyone from implementing Compilable except by implementing Contract.
mod private {
    pub trait ImplSeal {}

    /// Allow Contract to implement Compile
    impl ImplSeal for super::Compiled {}
    impl<'a, C> ImplSeal for C where C: super::AnyContract<'a> {}
}
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

impl<T> Compilable for T
where
    T: for<'a> AnyContract<'a>,
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
        let mut clause_accumulator = vec![];
        let mut ctv_to_tx = HashMap::new();
        let mut suggested_txs = HashMap::new();
        let self_ref = self.get_inner_ref();

        let finish_fns: Vec<_> = self
            .finish_fns()
            .iter()
            .filter_map(|x| guard_clauses.get(self_ref, *x))
            .collect();
        if finish_fns.len() > 0 {
            clause_accumulator.push(Clause::Threshold(1, finish_fns))
        }

        let then_fns = self
            .then_fns()
            .iter()
            .filter_map(|x| x())
            .map(|x| (UsesCTV::Yes, x.0, x.1(self_ref)));
        let finish_or_fns = self.finish_or_fns().iter().filter_map(|x| x()).map(|x| {
            (
                UsesCTV::No,
                x.guards(),
                x.fun()(self_ref, Default::default()),
            )
        });

        let mut amount_range = AmountRange::new();

        // If no guards and not CTV, then nothing gets added (not interpreted as Trivial True)
        // If CTV and no guards, just CTV added.
        // If CTV and guards, CTV & guards added.
        for (uses_ctv, guards, r_txtmpls) in then_fns.chain(finish_or_fns) {
            // it would be an error if any of r_txtmpls is an error instead of just an empty
            // iterator.
            let txtmpls = r_txtmpls?;
            // Don't use a threshold here because then miniscript will just re-compile it into the
            // And for again.
            let mut option_guard = guards
                .iter()
                .filter_map(|x| guard_clauses.get(self_ref, *x))
                .fold(None, |option_guard, guard| {
                    Some(match option_guard {
                        None => guard,
                        Some(guards) => Clause::And(vec![guards, guard]),
                    })
                });

            let mut txtmpl_clauses = txtmpls
                .map(|r_txtmpl| {
                    let txtmpl = r_txtmpl?;
                    let h = txtmpl.hash();
                    let txtmpl = match uses_ctv {
                        UsesCTV::Yes => &mut ctv_to_tx,
                        UsesCTV::No => &mut suggested_txs,
                    }
                    .entry(h)
                    .or_insert(txtmpl);
                    amount_range.update_range(txtmpl.total_amount());
                    Ok(Clause::TxTemplate(h))
                })
                // Forces any error to abort the whole thing
                .collect::<Result<Vec<_>, _>>()?;
            match uses_ctv {
                UsesCTV::Yes => {
                    let hashes = match txtmpl_clauses.len() {
                        0 => {
                            return Err(CompilationError::TerminateCompilation);
                        }
                        1 => {
                            // Safe because size must be > 0
                            txtmpl_clauses.pop().unwrap()
                        }
                        n => Clause::Threshold(1, txtmpl_clauses),
                    };
                    option_guard = Some(if let Some(guard) = option_guard {
                        Clause::And(vec![guard, hashes])
                    } else {
                        hashes
                    });
                }
                UsesCTV::No => {}
            }
            if let Some(guard) = option_guard {
                clause_accumulator.push(guard);
            }
        }

        let policy = match clause_accumulator.len() {
            0 => return Err(CompilationError::TerminateCompilation),
            1 => clause_accumulator.pop().unwrap(),
            _ => Clause::Threshold(1, clause_accumulator),
        };

        let descriptor = Descriptor::Wsh(
            policy
                .compile()
                .map_err(|_| CompilationError::TerminateCompilation)?,
        );

        Ok(Compiled {
            ctv_to_tx,
            suggested_txs,
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
