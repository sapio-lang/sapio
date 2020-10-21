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

/// A Guard is a function which generates some condition that must be met to unlock a script.
/// If bool = true, the computation of the guard is cached, which is useful if e.g. Guard
/// must contact a remote server or it should be the same across calls *for a given contract
/// instance*.
pub struct Guard<ContractSelf>(pub fn(&ContractSelf) -> Clause, pub bool);

/// A List of Guards, for convenience
pub type GuardList<'a, T> = &'a [Option<Guard<T>>];

/// A ThenFunc takes a list of Guards and a TxTmplIt generator.  Each TxTmpl returned from the
/// ThenFunc is Covenant Permitted only if the AND of all guards is satisfied.
pub struct ThenFunc<'a, ContractSelf: 'a>(
    pub GuardList<'a, ContractSelf>,
    pub fn(&ContractSelf) -> TxTmplIt,
);

/// A function which by default finishes, but may receive some context object which can induce the
/// generation of additional transactions (as a suggestion)
///
/// FinishOrFuncNew is used to construct a FinishOrFunc to workaround the const_fn restrictions on
/// function arguments.
pub struct FinishOrFunc<'a, ContractSelf: 'a, Extra> {
    ffn: FinishOrFuncNew<'a, ContractSelf, Extra>,
}

/// Workaround of const_fn not accepting arguments that are fns, otherwise this would be inlined
/// inside of FinishOrFunc.
pub struct FinishOrFuncNew<'a, ContractSelf: 'a, Extra>(
    pub GuardList<'a, ContractSelf>,
    pub fn(&'a ContractSelf, Option<&'a Extra>) -> TxTmplIt<'a>,
);

impl<'a, ContractSelf: 'a, Extra> FinishOrFunc<'a, ContractSelf, Extra> {
    /// Accessor to get the function of a FinishOrFunc
    fn fun(&self) -> fn(&'a ContractSelf, Option<&'a Extra>) -> TxTmplIt<'a> {
        self.ffn.1
    }

    /// Accessor to get the guards of a FinishOrFunc
    fn guards(&self) -> &'a [Option<Guard<ContractSelf>>] {
        self.ffn.0
    }
}

/// Because From is a Trait, it cannot be const. Therefore we provide our own  non-trait method.
impl<'a, ContractSelf: 'a, Extra> FinishOrFuncNew<'a, ContractSelf, Extra> {
    pub const fn into(self) -> FinishOrFunc<'a, ContractSelf, Extra> {
        FinishOrFunc { ffn: self }
    }
}

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

/// The def macro is used to define the list of pathways in a contract
#[macro_export]
macro_rules! def {
    {then $(,$a:expr)*} => {
        const THEN_FNS: &'a [Option<ThenFunc<'a, Self>>] = &[$($a,)*];
    };
    [state $i:ident]  => {
        type StatefulArguments = $i;
    };

    [state]  => {
        type StatefulArguments;
    };
    {updatable<$($i:ident)?> $(,$a:expr)*} => {
        const FINISH_OR_FUNCS: &'a [Option<FinishOrFunc<'a, Self, Self::StatefulArguments>>] = &[$($a,)*];
        def![state $($i)?];
    };
    {finish $(,$a:expr)*} => {
        const FINISH_FNS: &'a [Option<Guard<Self>>] = &[$($a,)*];
    };


}

/// The then macro is used to define a `ThenFunc`
#[macro_export]
macro_rules! then {
    {$name:ident $a:tt |$s:ident| $b:block } => {
        const $name: Option<ThenFunc<'a, Self>> = Some(ThenFunc(&$a, |$s: &Self| $b));
    };
    {$name:ident |$s:ident| $b:block } => { then!{$name [] |$s| $b } };
}

/// The then macro is used to define a `FinishFunc` or a `FinishOrFunc`
#[macro_export]
macro_rules! finish {
    {$name:ident $a:tt |$s:ident, $o:ident| $b:block } => {
        const $name: Option<FinishOrFunc<'a, Self, Args>> = Some(FinishOrFuncNew(&$a, |$s: &Self, $o: Option<&_>| $b) .into());
    };
    {$name:ident $a:tt} => {
        finish!($name $a |s, o| {Box::new(std::iter::empty())});
    };
}

/// The guard macro is used to define a `Guard`. Guards may be cached or uncached.
#[macro_export]
macro_rules! guard {
    {$name:ident |$s:ident| $b:block} => {
                                             const $name: Option<Guard<Self>> = Some(Guard( |$s: &Self| $b, false,));

                                         };
    {cached $name:ident |$s:ident| $b:block} => { const $name: Option<Guard<Self>> = Some(Guard( |$s: &Self| $b, true,)); };
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
        let guard_clauses = {
            let mut guard_clauses: HashMap<usize, Clause> = HashMap::new();
            let guards2 = Self::FINISH_OR_FUNCS
                .iter()
                .filter_map(|x| x.as_ref().map(|y| y.guards().iter()));
            let _guards3 = Self::FINISH_FNS.iter();
            for guards in Self::THEN_FNS
                .iter()
                .filter_map(|x| x.as_ref().map(|y| y.0.iter()))
                .chain(guards2)
            {
                for guard in guards.filter_map(|x| x.as_ref()) {
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
            .filter_map(|x| x.as_ref())
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
            .filter_map(|x| x.as_ref())
            .map(|x| (UsesCTV::Yes, x.0, x.1(self)));
        let finish_or_fns = Self::FINISH_OR_FUNCS
            .iter()
            .filter_map(|x| x.as_ref())
            .map(|x| (UsesCTV::No, x.ffn.0, x.fun()(self, Default::default())));

        let mut amount_range = AmountRange::new();
        for (uses_ctv, guards, txtmpls) in then_fns.chain(finish_or_fns) {
            // If no guards and not CTV, then nothing gets added (not interpreted as Trivial True)
            // If CTV and no guards, just CTV added.
            // If CTV and guards, CTV & guards added.
            let mut option_guard = guards
                .iter()
                .filter_map(|x| x.as_ref())
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
