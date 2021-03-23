// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Functionality comprising the language base, macros, and compiler internals.
use crate::template::Template as TransactionTemplate;

#[macro_use]
pub mod macros;
pub mod actions;
pub mod compiler;
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
pub struct DynamicContract<'a, T, S>
where
    S: 'a,
{
    /// the list of `ThenFunc` for this contract.
    pub then: Vec<fn() -> Option<actions::ThenFunc<'a, S>>>,
    /// the list of `FinishOrFunc` for this contract.
    pub finish_or: Vec<fn() -> Option<actions::FinishOrFunc<'a, S, T>>>,
    /// the list of `Guard` for this contract to finish.
    pub finish: Vec<fn() -> Option<actions::Guard<S>>>,
    /// The contract data argument to pass to functions
    pub data: S,
}

impl<T, S> AnyContract for DynamicContract<'_, T, S> {
    type StatefulArguments = T;
    type Ref = S;
    fn then_fns<'a>(&'a self) -> &'a [fn() -> Option<actions::ThenFunc<'a, S>>]
    where
        Self::Ref: 'a,
    {
        &self.then[..]
    }
    fn finish_or_fns<'a>(
        &'a self,
    ) -> &'a [fn() -> Option<actions::FinishOrFunc<'a, S, Self::StatefulArguments>>] {
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
    /// The parameter pack type for `FinishOrFunc`s.
    type StatefulArguments;
    /// A Reference which can be extracted to the contract argument data
    /// For some types, Ref == Self, and for other types Ref may point to a member.
    /// This enables `DynamicContract` and `Contract` to impl `AnyContract`, as well
    /// as more exotic types.
    type Ref;
    /// obtain a reference to the `ThenFunc` list.
    fn then_fns<'a>(&'a self) -> &'a [fn() -> Option<actions::ThenFunc<'a, Self::Ref>>]
    where
        Self::Ref: 'a;

    /// obtain a reference to the `FinishOrFunc` list.
    fn finish_or_fns<'a>(
        &'a self,
    ) -> &'a [fn() -> Option<actions::FinishOrFunc<'a, Self::Ref, Self::StatefulArguments>>];
    /// obtain a reference to the `Guard` list.
    fn finish_fns<'a>(&'a self) -> &'a [fn() -> Option<actions::Guard<Self::Ref>>];
    /// obtain a reference to `Self::Ref` type.
    fn get_inner_ref<'a>(&'a self) -> &'a Self::Ref;
}

impl<C> AnyContract for C
where
    C: Contract + Sized,
{
    type StatefulArguments = C::StatefulArguments;
    type Ref = Self;
    fn then_fns<'a>(&'a self) -> &'a [fn() -> Option<actions::ThenFunc<'a, Self::Ref>>]
    where
        Self::Ref: 'a,
    {
        Self::THEN_FNS
    }
    fn finish_or_fns<'a>(
        &'a self,
    ) -> &'a [fn() -> Option<actions::FinishOrFunc<'a, Self::Ref, Self::StatefulArguments>>] {
        Self::FINISH_OR_FUNCS
    }
    fn finish_fns<'a>(&'a self) -> &'a [fn() -> Option<actions::Guard<Self::Ref>>] {
        Self::FINISH_FNS
    }
    fn get_inner_ref<'a>(&'a self) -> &Self::Ref {
        self
    }
}
