use crate::template::Template as TransactionTemplate;

#[macro_use]
pub mod macros;
pub mod actions;
pub mod compiler;
pub mod emulator;
pub mod error;
pub mod object;
pub use error::CompilationError;
pub mod context;
pub use context::Context;

use bitcoin::util::amount::Amount;
pub use compiler::Compilable;
pub use object::Object as Compiled;

/// An Iterator which yields TransactionTemplates.
/// It is boxed to permit flexibility when returning.
pub type TxTmplIt = Result<
    Box<dyn Iterator<Item = Result<TransactionTemplate, CompilationError>>>,
    CompilationError,
>;

/// A catch-all type for any function that is a FinishOrFunc.
/// Unfortunately, because type signatures must all match, it's not
/// possible to have differing types across FinishOrFunc for a contract at compile time.
/// Use an enum if need be.
///
/// TODO: use associated-type defaults here!
pub trait Contract
where
    Self: Sized + 'static,
    Option<Self::StatefulArguments>: Default,
{
    //! Main Contract Trait
    declare! {then}
    declare! { updatable<> }
    declare! {finish}
}

/// DynamicContract wraps a struct S with a set of methods (that can be constructed dynamically)
/// to form a contract. DynamicContract owns all its methods.
pub struct DynamicContract<T, S>
where
    S: 'static,
{
    pub then: Vec<fn() -> Option<actions::ThenFunc<S>>>,
    pub finish_or: Vec<fn() -> Option<actions::FinishOrFunc<S, T>>>,
    pub finish: Vec<fn() -> Option<actions::Guard<S>>>,
    pub data: S,
}

impl<T, S> AnyContract for DynamicContract<T, S> {
    type StatefulArguments = T;
    type Ref = S;
    fn then_fns<'a>(&'a self) -> &'a [fn() -> Option<actions::ThenFunc<S>>] {
        &self.then[..]
    }
    fn finish_or_fns<'a>(
        &'a self,
    ) -> &'a [fn() -> Option<actions::FinishOrFunc<S, Self::StatefulArguments>>] {
        &self.finish_or[..]
    }
    fn finish_fns<'a>(&'a self) -> &'a [fn() -> Option<actions::Guard<S>>] {
        &self.finish[..]
    }
    fn get_inner_ref<'a>(&self) -> &Self::Ref {
        &self.data
    }
}
/// AnyContract is a generic API for types which can be compiled, encapsulating default static
/// Contracts as well as DynamicContracts/DynamicContractRefs.
///
/// This assists in abstracting the layout/internals away from something that can be compiled.
pub trait AnyContract
where
    Self: Sized,
{
    type StatefulArguments;
    type Ref;
    fn then_fns<'a>(&'a self) -> &'a [fn() -> Option<actions::ThenFunc<Self::Ref>>];
    fn finish_or_fns<'a>(
        &'a self,
    ) -> &'a [fn() -> Option<actions::FinishOrFunc<Self::Ref, Self::StatefulArguments>>];
    fn finish_fns<'a>(&'a self) -> &'a [fn() -> Option<actions::Guard<Self::Ref>>];
    fn get_inner_ref<'a>(&'a self) -> &'a Self::Ref;
}

impl<C> AnyContract for C
where
    C: Contract + Sized,
{
    type StatefulArguments = C::StatefulArguments;
    type Ref = Self;
    fn then_fns<'a>(&'a self) -> &'a [fn() -> Option<actions::ThenFunc<Self::Ref>>] {
        Self::THEN_FNS
    }
    fn finish_or_fns<'a>(
        &'a self,
    ) -> &'a [fn() -> Option<actions::FinishOrFunc<Self::Ref, Self::StatefulArguments>>] {
        Self::FINISH_OR_FUNCS
    }
    fn finish_fns<'a>(&'a self) -> &'a [fn() -> Option<actions::Guard<Self::Ref>>] {
        Self::FINISH_FNS
    }
    fn get_inner_ref<'a>(&'a self) -> &Self::Ref {
        self
    }
}
