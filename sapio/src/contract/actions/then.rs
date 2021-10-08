// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Functionality for a function that uses CTV
use super::Context;
use super::TxTmplIt;
use crate::contract::actions::ConditionallyCompileIfList;
use crate::contract::actions::GuardList;
use std::sync::Arc;

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
    pub func: fn(&ContractSelf, Context) -> TxTmplIt,
    /// name derived from Function Name.
    pub name: Arc<String>,
}
