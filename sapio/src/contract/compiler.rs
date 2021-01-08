use super::AnyContract;
use super::CompilationError;
use super::Compiled;
use super::Context;
use crate::util::amountrange::AmountRange;

use super::actions::Guard;
use crate::clause::Clause;
use ::miniscript::*;
use std::collections::HashMap;

enum CacheEntry<T> {
    Cached(Clause),
    Fresh(fn(&T, &Context) -> Clause),
}

/// GuardCache assists with caching the computation of guard functions
/// during compilation.
struct GuardCache<T> {
    cache: HashMap<usize, Option<CacheEntry<T>>>,
}
impl<T> GuardCache<T> {
    fn new() -> Self {
        GuardCache {
            cache: HashMap::new(),
        }
    }
    fn create_entry(g: Option<Guard<T>>, t: &T, ctx: &Context) -> Option<CacheEntry<T>> {
        Some(match g? {
            Guard::Cache(f) => CacheEntry::Cached(f(t, ctx)),
            Guard::Fresh(f) => CacheEntry::Fresh(f),
        })
    }
    fn get(&mut self, t: &T, f: fn() -> Option<Guard<T>>, ctx: &Context) -> Option<Clause> {
        Some(
            match self
                .cache
                .entry(f as usize)
                .or_insert_with(|| Self::create_entry(f(), t, ctx))
                .as_ref()?
            {
                CacheEntry::Cached(s) => s.clone(),
                CacheEntry::Fresh(f) => f(t, ctx),
            },
        )
    }
}

/// private::ImplSeal prevents anyone from implementing Compilable except by implementing Contract.
mod private {
    pub trait ImplSeal {}

    /// Allow Contract to implement Compile
    impl ImplSeal for super::Compiled {}
    impl<'a, C> ImplSeal for C where C: super::AnyContract {}
}
/// Compilable is a trait for anything which can be compiled
pub trait Compilable: private::ImplSeal {
    fn compile(&self, ctx: &Context) -> Result<Compiled, CompilationError>;
}

/// Implements a basic identity
impl Compilable for Compiled {
    fn compile(&self, _ctx: &Context) -> Result<Compiled, CompilationError> {
        Ok(self.clone())
    }
}

impl<'a, T> Compilable for T
where
    T: AnyContract + 'a,
    T::Ref: 'a,
{
    /// The main Compilation Logic for a Contract.
    /// TODO: Better Document Semantics
    fn compile(&self, ctx: &Context) -> Result<Compiled, CompilationError> {
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
            .filter_map(|x| guard_clauses.get(self_ref, *x, ctx))
            .collect();
        if finish_fns.len() > 0 {
            clause_accumulator.push(Clause::Threshold(1, finish_fns))
        }

        let then_fns = self
            .then_fns()
            .iter()
            .filter_map(|x| x())
            .map(|x| (UsesCTV::Yes, x.guard, (x.func)(self_ref, ctx)));
        let finish_or_fns = self.finish_or_fns().iter().filter_map(|x| x()).map(|x| {
            (
                UsesCTV::No,
                x.guard,
                (x.func)(self_ref, ctx, Default::default()),
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
                .filter_map(|x| guard_clauses.get(self_ref, *x, ctx))
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
                    ctx.ctv_emulator(h)
                })
                // Forces any error to abort the whole thing
                .collect::<Result<Vec<_>, CompilationError>>()?;
            match uses_ctv {
                UsesCTV::Yes => {
                    let hashes = match txtmpl_clauses.len() {
                        0 => {
                            return Err(CompilationError::MissingTemplates);
                        }
                        1 => txtmpl_clauses
                            .pop()
                            .expect("Length of txtmpl_clauses must be at least 1"),
                        _n => Clause::Threshold(1, txtmpl_clauses),
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
            0 => return Err(CompilationError::EmptyPolicy),
            1 => clause_accumulator
                .pop()
                .expect("Length of policy must be at least 1"),
            _ => Clause::Threshold(1, clause_accumulator),
        };

        let miniscript = policy.compile().map_err(Into::<CompilationError>::into)?;
        let address = bitcoin::Address::p2wsh(&miniscript.encode(), bitcoin::Network::Bitcoin);
        let descriptor = Some(Descriptor::Wsh(miniscript));

        Ok(Compiled {
            ctv_to_tx,
            suggested_txs,
            address,
            descriptor,
            policy: Some(policy),
            amount_range,
        })
    }
}
