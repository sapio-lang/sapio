use crate::txn::Template as TransactionTemplate;

#[macro_use]
pub mod macros;
pub mod actions;
pub mod compiler;
pub mod object;

pub use compiler::Compilable;
pub use object::Object as Compiled;

use std::error::Error;
use std::fmt;
#[derive(Debug)]
pub enum CompilationError {
    TerminateCompilation,
    MissingTemplates,
    EmptyPolicy,
    Miniscript(miniscript::policy::compiler::CompilerError),
}

impl From<miniscript::policy::compiler::CompilerError> for CompilationError {
    fn from(v: miniscript::policy::compiler::CompilerError) -> Self {
        CompilationError::Miniscript(v)
    }
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
    declare! {then}
    declare! { updatable<> }
    declare! {finish}
}

/// DynamicContract wraps a struct S with a set of methods (that can be constructed dynamically)
/// to form a contract. DynamicContract owns all its methods.
struct DynamicContract<'a, T, S> {
    then: Vec<fn() -> Option<actions::ThenFunc<'a, S>>>,
    finish_or: Vec<fn() -> Option<actions::FinishOrFunc<'a, S, T>>>,
    finish: Vec<fn() -> Option<actions::Guard<S>>>,
    data: S,
}

/// Coerce DynamicContract into a DynamicContractRef, which does not own its methods.
impl<'a, T, S> From<&'a DynamicContract<'a, T, S>> for DynamicContractRef<'a, T, S> {
    fn from(d: &'a DynamicContract<'a, T, S>) -> Self {
        DynamicContractRef {
            then: &d.then[..],
            finish_or: &d.finish_or[..],
            finish: &d.finish[..],
            data: &d.data,
        }
    }
}

impl<'a, T, S> AnyContract<'a> for DynamicContract<'a, T, S> {
    type StatefulArguments = T;
    type Ref = S;
    fn then_fns(&'a self) -> &'a [fn() -> Option<actions::ThenFunc<'a, S>>] {
        &self.then[..]
    }
    fn finish_or_fns(
        &'a self,
    ) -> &'a [fn() -> Option<actions::FinishOrFunc<'a, S, Self::StatefulArguments>>] {
        &self.finish_or[..]
    }
    fn finish_fns(&'a self) -> &'a [fn() -> Option<actions::Guard<S>>] {
        &self.finish[..]
    }
    fn get_inner_ref(&self) -> &Self::Ref {
        &self.data
    }
}
/// Like DynamicContract, but without owning the methods slice or underlying data.
struct DynamicContractRef<'a, T, S> {
    then: &'a [fn() -> Option<actions::ThenFunc<'a, S>>],
    finish_or: &'a [fn() -> Option<actions::FinishOrFunc<'a, S, T>>],
    finish: &'a [fn() -> Option<actions::Guard<S>>],
    data: &'a S,
}
impl<'a, T, S> AnyContract<'a> for DynamicContractRef<'a, T, S> {
    type StatefulArguments = T;
    type Ref = S;
    fn then_fns(&self) -> &'a [fn() -> Option<actions::ThenFunc<'a, S>>] {
        self.then
    }
    fn finish_or_fns(
        &self,
    ) -> &'a [fn() -> Option<actions::FinishOrFunc<'a, S, Self::StatefulArguments>>] {
        self.finish_or
    }
    fn finish_fns(&self) -> &'a [fn() -> Option<actions::Guard<S>>] {
        self.finish
    }
    fn get_inner_ref(&self) -> &Self::Ref {
        self.data
    }
}

/// AnyContract is a generic API for types which can be compiled, encapsulating default static
/// Contracts as well as DynamicContracts/DynamicContractRefs.
///
/// This assists in abstracting the layout/internals away from something that can be compiled.
pub trait AnyContract<'a>
where
    Self: Sized + 'a,
{
    type StatefulArguments;
    type Ref;
    fn then_fns(&'a self) -> &'a [fn() -> Option<actions::ThenFunc<'a, Self::Ref>>];
    fn finish_or_fns(
        &'a self,
    ) -> &'a [fn() -> Option<actions::FinishOrFunc<'a, Self::Ref, Self::StatefulArguments>>];
    fn finish_fns(&'a self) -> &'a [fn() -> Option<actions::Guard<Self::Ref>>];
    fn get_inner_ref(&'a self) -> &'a Self::Ref;
}

impl<'a, C, T> AnyContract<'a> for C
where
    C: Contract<'a, StatefulArguments = T> + Sized,
{
    type StatefulArguments = T;
    type Ref = Self;
    fn then_fns(&self) -> &'a [fn() -> Option<actions::ThenFunc<'a, Self::Ref>>] {
        Self::THEN_FNS
    }
    fn finish_or_fns(
        &self,
    ) -> &'a [fn() -> Option<actions::FinishOrFunc<'a, Self::Ref, Self::StatefulArguments>>] {
        Self::FINISH_OR_FUNCS
    }
    fn finish_fns(&self) -> &'a [fn() -> Option<actions::Guard<Self::Ref>>] {
        Self::FINISH_FNS
    }
    fn get_inner_ref(&self) -> &Self::Ref {
        self
    }
}
