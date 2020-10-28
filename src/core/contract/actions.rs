use super::TxTmplIt;
use crate::clause::Clause;
/// A Guard is a function which generates some condition that must be met to unlock a script.
/// If bool = true, the computation of the guard is cached, which is useful if e.g. Guard
/// must contact a remote server or it should be the same across calls *for a given contract
/// instance*.
pub enum Guard<ContractSelf> {
    Cache(fn(&ContractSelf) -> Clause),
    Fresh(fn(&ContractSelf) -> Clause),
}

/// A List of Guards, for convenience
pub type GuardList<'a, T> = &'a [fn() -> Option<Guard<T>>];

/// A ThenFunc takes a list of Guards and a TxTmplIt generator.  Each TxTmpl returned from the
/// ThenFunc is Covenant Permitted only if the AND of all guards is satisfied.
pub struct ThenFunc<'a, ContractSelf: 'a> {
    pub guard: GuardList<'a, ContractSelf>,
    pub func: fn(&ContractSelf) -> TxTmplIt,
}

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
pub struct FinishOrFuncNew<'a, ContractSelf: 'a, Extra> {
    pub guard: GuardList<'a, ContractSelf>,
    pub func: fn(&'a ContractSelf, Option<&'a Extra>) -> TxTmplIt<'a>,
}

impl<'a, ContractSelf: 'a, Extra> FinishOrFunc<'a, ContractSelf, Extra> {
    /// Accessor to get the function of a FinishOrFunc
    pub fn fun(&self) -> fn(&'a ContractSelf, Option<&'a Extra>) -> TxTmplIt<'a> {
        self.ffn.func
    }

    /// Accessor to get the guards of a FinishOrFunc
    pub fn guards(&self) -> &'a [fn() -> Option<Guard<ContractSelf>>] {
        self.ffn.guard
    }
}

/// Because From is a Trait, it cannot be const. Therefore we provide our own  non-trait method.
impl<'a, ContractSelf: 'a, Extra> FinishOrFuncNew<'a, ContractSelf, Extra> {
    pub const fn into(self) -> FinishOrFunc<'a, ContractSelf, Extra> {
        FinishOrFunc { ffn: self }
    }
}
