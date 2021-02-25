//! The different types of functionality a contract can define.
use super::Context;
use super::TxTmplIt;
use sapio_base::Clause;
/// A Guard is a function which generates some condition that must be met to unlock a script.
/// If bool = true, the computation of the guard is cached, which is useful if e.g. Guard
/// must contact a remote server or it should be the same across calls *for a given contract
/// instance*.
pub enum Guard<ContractSelf> {
    /// Cache Variant should only be called one time per contract and the result saved
    Cache(fn(&ContractSelf, &Context) -> Clause),
    /// Fresh Variant may be called repeatedly
    Fresh(fn(&ContractSelf, &Context) -> Clause),
}

/// A List of Guards, for convenience
pub type GuardList<'a, T> = &'a [fn() -> Option<Guard<T>>];

/// A ThenFunc takes a list of Guards and a TxTmplIt generator.  Each TxTmpl returned from the
/// ThenFunc is Covenant Permitted only if the AND of all guards is satisfied.
pub struct ThenFunc<'a, ContractSelf: 'a> {
    /// Guards returns Clauses -- if any -- before the internal func's returned
    /// TxTmpls should execute on-chain
    pub guard: GuardList<'a, ContractSelf>,
    /// func returns an iterator of possible transactions
    /// Implementors should aim to return as few `TxTmpl`s as possible for enhanced
    /// semantics, preferring to split across multiple `ThenFunc`'s
    pub func: fn(&ContractSelf, &Context) -> TxTmplIt,
}

/// A function which by default finishes, but may receive some context object which can induce the
/// generation of additional transactions (as a suggestion)
pub struct FinishOrFunc<'a, ContractSelf: 'a, Extra> {
    /// Guards returns Clauses -- if any -- before the coins should be unlocked
    pub guard: GuardList<'a, ContractSelf>,
    /// func returns an iterator of possible transactions
    /// Implementors should aim to return as few `TxTmpl`s as possible for enhanced
    /// semantics, preferring to split across multiple `FinishOrFunc`'s.
    /// These `TxTmpl`s are non-binding, merely suggested.
    pub func: fn(&ContractSelf, &Context, Option<&Extra>) -> TxTmplIt,
}
