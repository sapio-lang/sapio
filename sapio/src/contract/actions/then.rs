// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Functionality for a function that uses CTV
use super::CompilationError;
use super::Context;
use super::TxTmplIt;
use crate::contract::actions::ConditionallyCompileIfList;
use crate::contract::actions::GuardList;
use crate::contract::actions::{FinishOrFunc, WebAPIDisabled};
use std::marker::PhantomData;
use std::sync::Arc;

/// ThenFuncTypeTag is used as the args type of a ThenFunc
pub struct ThenFuncTypeTag(pub(crate) ());

impl ThenFuncTypeTag {
    /// coerce of Self maps onto Self
    pub fn coerce_args(f: Self) -> Result<Self, CompilationError> {
        Ok(f)
    }
}

/// Alias for representation of ThenFunc as FinishOrFunc
pub type ThenFuncAsFinishOrFunc<'a, ContractSelf> =
    FinishOrFunc<'a, ContractSelf, ThenFuncTypeTag, ThenFuncTypeTag, WebAPIDisabled>;

/// A ThenFunc takes a list of Guards and a TxTmplIt generator.  Each TxTmpl returned from the
/// ThenFunc is Covenant Permitted only if the AND of all guards is satisfied.
pub struct ThenFunc<'a, ContractSelf> {
    /// Guards returns Clauses -- if any -- before the internal func's returned
    /// TxTmpls should execute on-chain
    pub guard: GuardList<'a, ContractSelf>,
    /// conditional_compile_if returns ConditionallyCompileType to determine if a function
    /// should be included.
    pub conditional_compile_if: ConditionallyCompileIfList<'a, ContractSelf>,
    /// func returns an iterator of possible transactions
    /// Implementors should aim to return as few `TxTmpl`s as possible for enhanced
    /// semantics, preferring to split across multiple `ThenFunc`'s
    pub func: fn(&ContractSelf, Context, ThenFuncTypeTag) -> TxTmplIt,
    /// name derived from Function Name.
    pub name: Arc<String>,
}

impl<'a, ContractSelf> From<ThenFunc<'a, ContractSelf>>
    for ThenFuncAsFinishOrFunc<'a, ContractSelf>
{
    fn from(f: ThenFunc<'a, ContractSelf>) -> Self {
        FinishOrFunc {
            guard: f.guard,
            conditional_compile_if: f.conditional_compile_if,
            func: f.func,
            name: f.name,
            coerce_args: ThenFuncTypeTag::coerce_args,
            schema: None,
            f: PhantomData::default(),
        }
    }
}
