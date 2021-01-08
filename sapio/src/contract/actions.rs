use super::Context;
use super::TxTmplIt;
use crate::clause::Clause;
/// A Guard is a function which generates some condition that must be met to unlock a script.
/// If bool = true, the computation of the guard is cached, which is useful if e.g. Guard
/// must contact a remote server or it should be the same across calls *for a given contract
/// instance*.
pub enum Guard<ContractSelf> {
    Cache(fn(&ContractSelf, &Context) -> Clause),
    Fresh(fn(&ContractSelf, &Context) -> Clause),
}

/// A List of Guards, for convenience
pub type GuardList<'a, T: 'a> = &'a [fn() -> Option<Guard<T>>];

/// A ThenFunc takes a list of Guards and a TxTmplIt generator.  Each TxTmpl returned from the
/// ThenFunc is Covenant Permitted only if the AND of all guards is satisfied.
pub struct ThenFunc<'a, ContractSelf: 'a> {
    pub guard: GuardList<'a, ContractSelf>,
    pub func: fn(&ContractSelf, &Context) -> TxTmplIt,
}

/// A function which by default finishes, but may receive some context object which can induce the
/// generation of additional transactions (as a suggestion)
pub struct FinishOrFunc<'a, ContractSelf: 'a, Extra> {
    pub guard: GuardList<'a, ContractSelf>,
    pub func: fn(&ContractSelf, &Context, Option<&Extra>) -> TxTmplIt,
}
