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
use bitcoin::amount::Amount;
use bitcoin::XOnlyPublicKey;
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
/// A contract's fixed policies and independently typed transaction actions.
pub trait Contract: Sized + 'static {
    /// Explicitly exported actions. An absent factory represents an optional
    /// interface action that this implementation does not provide.
    const ACTIONS: &'static [actions::ActionFactory<Self>] = &[];
    /// Independently sufficient spending policies.
    const FINISH_FNS: &'static [fn() -> Option<actions::Guard<Self>>] = &[];
    /// Generate descriptive metadata for this contract object.
    fn metadata(&self, _ctx: Context) -> Result<ObjectMetadata, CompilationError> {
        Ok(Default::default())
    }
    /// Minimum balance needed by this contract.
    fn ensure_amount(&self, _ctx: Context) -> Result<Amount, CompilationError> {
        Ok(Amount::ZERO)
    }
    /// Pin an already authorized bare key branch as the Taproot internal key.
    /// Selecting a key never grants new authority. The compiler removes its
    /// redundant bare-key leaf once it becomes the key path.
    fn pinned_internal_key(
        &self,
        _ctx: &Context,
    ) -> Result<Option<XOnlyPublicKey>, CompilationError> {
        Ok(None)
    }
}

/// A contract whose explicitly exported actions are assembled at runtime.
pub struct DynamicContract<S> {
    /// Independently typed action factories.
    pub actions: Vec<actions::ActionFactory<S>>,
    /// Independent spending policies.
    pub finish: Vec<fn() -> Option<actions::Guard<S>>>,
    /// Descriptive metadata callback.
    pub metadata_f: Box<dyn Fn(&S, Context) -> Result<ObjectMetadata, CompilationError>>,
    /// Minimum funding callback.
    pub ensure_amount_f: Box<dyn Fn(&S, Context) -> Result<Amount, CompilationError>>,
    /// Contract data observed by callbacks.
    pub data: S,
}

/// Compiler interface shared by static and dynamically assembled contracts.
pub trait AnyContract: Sized {
    /// Contract data observed by callbacks.
    type Ref;
    /// Explicitly exported transaction actions.
    fn actions(&self) -> &[actions::ActionFactory<Self::Ref>];
    /// Independently sufficient spending policies.
    fn finish_fns(&self) -> &[fn() -> Option<actions::Guard<Self::Ref>>];
    /// Borrow the contract data.
    fn get_inner_ref(&self) -> &Self::Ref;
    /// Descriptive metadata.
    fn metadata(&self, ctx: Context) -> Result<ObjectMetadata, CompilationError>;
    /// Minimum funding required.
    fn ensure_amount(&self, ctx: Context) -> Result<Amount, CompilationError>;
    /// Select an independently authorized bare key as the internal key.
    fn pinned_internal_key(
        &self,
        _ctx: &Context,
    ) -> Result<Option<XOnlyPublicKey>, CompilationError> {
        Ok(None)
    }
}
impl<S> AnyContract for DynamicContract<S> {
    type Ref = S;
    fn actions(&self) -> &[actions::ActionFactory<S>] {
        &self.actions
    }
    fn finish_fns(&self) -> &[fn() -> Option<actions::Guard<S>>] {
        &self.finish
    }
    fn get_inner_ref(&self) -> &S {
        &self.data
    }
    fn metadata(&self, ctx: Context) -> Result<ObjectMetadata, CompilationError> {
        (self.metadata_f)(&self.data, ctx)
    }
    fn ensure_amount(&self, ctx: Context) -> Result<Amount, CompilationError> {
        (self.ensure_amount_f)(&self.data, ctx)
    }
}
impl<C: Contract> AnyContract for C {
    type Ref = Self;
    fn actions(&self) -> &[actions::ActionFactory<Self>] {
        Self::ACTIONS
    }
    fn finish_fns(&self) -> &[fn() -> Option<actions::Guard<Self>>] {
        Self::FINISH_FNS
    }
    fn get_inner_ref(&self) -> &Self {
        self
    }
    fn metadata(&self, ctx: Context) -> Result<ObjectMetadata, CompilationError> {
        Contract::metadata(self, ctx)
    }
    fn ensure_amount(&self, ctx: Context) -> Result<Amount, CompilationError> {
        Contract::ensure_amount(self, ctx)
    }
    fn pinned_internal_key(
        &self,
        ctx: &Context,
    ) -> Result<Option<XOnlyPublicKey>, CompilationError> {
        Contract::pinned_internal_key(self, ctx)
    }
}
