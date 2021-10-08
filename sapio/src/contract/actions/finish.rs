// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! a function type which is used to wrap a next transaction.
use super::CompilationError;
use super::Context;
use super::TxTmplIt;
use crate::contract::actions::ConditionallyCompileIfList;
use crate::contract::actions::GuardList;
use sapio_base::effects::EffectDBError;

use core::marker::PhantomData;
use schemars::schema::RootSchema;
use serde::Deserialize;
use std::sync::Arc;

/// A function which by default finishes, but may receive some context object which can induce the
/// generation of additional transactions (as a suggestion)
pub struct FinishOrFunc<'a, ContractSelf, StatefulArguments, SpecificArgs, WebAPIStatus> {
    /// StatefulArgs is needed to capture a general API for all calls, but SpecificArgs is required
    /// for a given function.
    pub coerce_args: fn(StatefulArguments) -> Result<SpecificArgs, CompilationError>,
    /// Guards returns Clauses -- if any -- before the coins should be unlocked
    pub guard: GuardList<'a, ContractSelf>,
    /// conditional_compile_if returns ConditionallyCompileType to determine if a function
    /// should be included.
    pub conditional_compile_if: ConditionallyCompileIfList<'a, ContractSelf>,
    /// func returns an iterator of possible transactions
    /// Implementors should aim to return as few `TxTmpl`s as possible for enhanced
    /// semantics, preferring to split across multiple `FinishOrFunc`'s.
    /// These `TxTmpl`s are non-binding, merely suggested.
    pub func: fn(&ContractSelf, Context, SpecificArgs) -> TxTmplIt,
    /// to be filled in if SpecificArgs has a schema, which it might not.
    /// because negative trait bounds do not exists, that is up to the
    /// implementation to decide if the trait exists.
    pub schema: Option<Arc<RootSchema>>,
    /// name derived from Function Name.
    pub name: Arc<String>,
    /// Type switch to enable/disable compilation with serialized fields
    /// (if negative trait bounds, could remove!)
    pub f: PhantomData<WebAPIStatus>,
}

/// This trait hides the generic parameter `SpecificArgs` in FinishOrFunc
/// through a trait object interface which enables FinishOrFuncs to have a
/// custom type per fucntion, so long as there is a way to convert from
/// StatefulArguments to SpecificArgs via coerce_args. By default, this is
/// presently done through `std::convert::TryInto::try_into`.
pub trait CallableAsFoF<ContractSelf, StatefulArguments> {
    /// Calls the internal function, should convert `StatefulArguments` to `SpecificArgs`.
    fn call(&self, cself: &ContractSelf, ctx: Context, o: StatefulArguments) -> TxTmplIt;
    /// Calls the internal function, should convert `StatefulArguments` to `SpecificArgs`.
    fn call_json(
        &self,
        cself: &ContractSelf,
        ctx: Context,
        o: serde_json::Value,
    ) -> Option<TxTmplIt>;
    /// Getter Method for internal field
    fn get_conditional_compile_if(&self) -> ConditionallyCompileIfList<'_, ContractSelf>;
    /// Getter Method for internal field
    fn get_guard(&self) -> GuardList<'_, ContractSelf>;
    /// Get the name for this function
    fn get_name(&self) -> &Arc<String>;
    /// Get the RootSchema for calling this with an update
    fn get_schema(&self) -> &Option<Arc<RootSchema>>;
}

/// Type Tag for FinishOrFunc Variant
pub struct WebAPIEnabled;
/// Type Tag for FinishOrFunc Variant
pub struct WebAPIDisabled;

impl<ContractSelf, StatefulArguments, SpecificArgs> CallableAsFoF<ContractSelf, StatefulArguments>
    for FinishOrFunc<'_, ContractSelf, StatefulArguments, SpecificArgs, WebAPIDisabled>
{
    fn call(&self, cself: &ContractSelf, ctx: Context, o: StatefulArguments) -> TxTmplIt {
        let args = (self.coerce_args)(o)?;
        (self.func)(cself, ctx, args)
    }
    fn call_json(
        &self,
        _cself: &ContractSelf,
        _ctx: Context,
        _o: serde_json::Value,
    ) -> Option<TxTmplIt> {
        None
    }
    fn get_conditional_compile_if(&self) -> ConditionallyCompileIfList<'_, ContractSelf> {
        self.conditional_compile_if
    }
    fn get_guard(&self) -> GuardList<'_, ContractSelf> {
        self.guard
    }
    fn get_name(&self) -> &Arc<String> {
        &self.name
    }
    fn get_schema(&self) -> &Option<Arc<RootSchema>> {
        &self.schema
    }
}

impl<ContractSelf, StatefulArguments, SpecificArgs> CallableAsFoF<ContractSelf, StatefulArguments>
    for FinishOrFunc<'_, ContractSelf, StatefulArguments, SpecificArgs, WebAPIEnabled>
where
    SpecificArgs: for<'de> Deserialize<'de>,
{
    fn call(&self, cself: &ContractSelf, ctx: Context, o: StatefulArguments) -> TxTmplIt {
        let args = (self.coerce_args)(o)?;
        (self.func)(cself, ctx, args)
    }
    fn call_json(
        &self,
        cself: &ContractSelf,
        ctx: Context,
        o: serde_json::Value,
    ) -> Option<TxTmplIt> {
        Some(
            serde_json::from_value(o)
                .map_err(EffectDBError::SerializationError)
                .map_err(CompilationError::EffectDBError)
                .and_then(|args| (self.func)(cself, ctx, args)),
        )
    }
    fn get_conditional_compile_if(&self) -> ConditionallyCompileIfList<'_, ContractSelf> {
        self.conditional_compile_if
    }
    fn get_guard(&self) -> GuardList<'_, ContractSelf> {
        self.guard
    }
    fn get_name(&self) -> &Arc<String> {
        &self.name
    }
    fn get_schema(&self) -> &Option<Arc<RootSchema>> {
        &self.schema
    }
}
