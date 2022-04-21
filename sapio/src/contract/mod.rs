// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Functionality comprising the language base, macros, and compiler internals.
use crate::contract::object::ObjectMetadata;
use crate::template::Template as TransactionTemplate;
#[macro_use]
pub mod macros;
pub mod abi;
// TODO: get rid of this rexport?
pub use abi::object;
pub mod actions;
pub mod compiler;
pub mod error;
pub use error::CompilationError;
pub mod context;
use bitcoin::util::amount::Amount;
pub use compiler::Compilable;
pub use context::Context;
pub use object::Object as Compiled;

/// An Iterator which yields TransactionTemplates.
/// It is boxed to permit flexibility when returning.
pub type TxTmplIt = Result<
    Box<dyn Iterator<Item = Result<TransactionTemplate, CompilationError>>>,
    CompilationError,
>;
/// Creates an empty TxTmplIt
pub fn empty() -> TxTmplIt {
    Ok(Box::new(std::iter::empty()))
}
/// A catch-all type for any function that is a FinishOrFunc.
/// Unfortunately, because type signatures must all match, it's not
/// possible to have differing types across FinishOrFunc for a contract at compile time.
/// Use an enum if need be.
///
/// TODO: use associated-type defaults here!
pub trait Contract
where
    Self: Sized + 'static,
    Self::StatefulArguments: StatefulArgumentsTrait,
{
    //! Main Contract Trait
    declare! {then}
    declare! { updatable<> }
    declare! {finish}
    /// Generate metadata for this contract object
    fn metadata(&self, ctx: Context) -> Result<ObjectMetadata, CompilationError> {
        Ok(Default::default())
    }

    /// minimum balance to have in this coin
    fn ensure_amount(&self, ctx: Context) -> Result<Amount, CompilationError> {
        Ok(Amount::from_sat(0))
    }
}

/// DynamicContract wraps a struct S with a set of methods (that can be constructed dynamically)
/// to form a contract. DynamicContract owns all its methods.
pub struct DynamicContract<'a, T, S> {
    /// the list of `ThenFunc` for this contract.
    pub then: Vec<fn() -> Option<actions::ThenFunc<'a, S>>>,
    /// the list of `FinishOrFunc` for this contract.
    pub finish_or: Vec<fn() -> Option<Box<dyn actions::CallableAsFoF<S, T>>>>,
    /// the list of `Guard` for this contract to finish.
    pub finish: Vec<fn() -> Option<actions::Guard<S>>>,
    /// A metadata generator function
    pub metadata_f: Box<dyn (Fn(&S, Context) -> Result<ObjectMetadata, CompilationError>)>,
    /// A min amount generator function
    pub ensure_amount_f: Box<dyn (Fn(&S, Context) -> Result<Amount, CompilationError>)>,

    /// The contract data argument to pass to functions
    pub data: S,
}

impl<T, S> AnyContract for DynamicContract<'_, T, S>
where
    T: StatefulArgumentsTrait,
{
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
    ) -> &'a [fn() -> Option<Box<dyn actions::CallableAsFoF<S, Self::StatefulArguments>>>] {
        &self.finish_or[..]
    }
    fn finish_fns<'a>(&'a self) -> &'a [fn() -> Option<actions::Guard<S>>] {
        &self.finish[..]
    }
    fn get_inner_ref<'a>(&self) -> &Self::Ref {
        &self.data
    }

    fn metadata<'a>(&'a self, ctx: Context) -> Result<ObjectMetadata, CompilationError> {
        (self.metadata_f)(self.get_inner_ref(), ctx)
    }

    fn ensure_amount<'a>(&'a self, ctx: Context) -> Result<Amount, CompilationError> {
        (self.ensure_amount_f)(self.get_inner_ref(), ctx)
    }
}

/// Catch all trait for things `StatefulArguments` must be required to do.
pub trait StatefulArgumentsTrait: Default {}
impl StatefulArgumentsTrait for () {}
impl<T> StatefulArgumentsTrait for Option<T> {}

/// AnyContract is a generic API for types which can be compiled, encapsulating default static
/// Contracts as well as DynamicContracts/DynamicContractRefs.
///
/// This assists in abstracting the layout/internals away from something that can be compiled.
pub trait AnyContract
where
    Self: Sized,
{
    /// The parameter pack type for `FinishOrFunc`s.
    type StatefulArguments: StatefulArgumentsTrait;
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
    ) -> &'a [fn() -> Option<Box<dyn actions::CallableAsFoF<Self::Ref, Self::StatefulArguments>>>];
    /// obtain a reference to the `Guard` list.
    fn finish_fns<'a>(&'a self) -> &'a [fn() -> Option<actions::Guard<Self::Ref>>];
    /// obtain a reference to `Self::Ref` type.
    fn get_inner_ref<'a>(&'a self) -> &'a Self::Ref;
    /// Generate the metadata
    fn metadata<'a>(&'a self, ctx: Context) -> Result<ObjectMetadata, CompilationError>;
    /// Minimum Amount
    fn ensure_amount<'a>(&'a self, ctx: Context) -> Result<Amount, CompilationError>;
}

impl<C> AnyContract for C
where
    C: Contract + Sized,
    C::StatefulArguments: StatefulArgumentsTrait,
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
    ) -> &'a [fn() -> Option<Box<dyn actions::CallableAsFoF<Self::Ref, Self::StatefulArguments>>>]
    {
        Self::FINISH_OR_FUNCS
    }
    fn finish_fns<'a>(&'a self) -> &'a [fn() -> Option<actions::Guard<Self::Ref>>] {
        Self::FINISH_FNS
    }
    fn get_inner_ref<'a>(&'a self) -> &Self::Ref {
        self
    }

    fn metadata<'a>(&'a self, ctx: Context) -> Result<ObjectMetadata, CompilationError> {
        Self::Ref::metadata(self, ctx)
    }
    fn ensure_amount<'a>(&'a self, ctx: Context) -> Result<Amount, CompilationError> {
        Self::Ref::ensure_amount(self, ctx)
    }
}
