// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! a decorator type which is used to generate a spending condition
use super::Context;

use sapio_base::Clause;
/// A Guard is a function which generates some condition that must be met to unlock a script.
/// If bool = true, the computation of the guard is cached, which is useful if e.g. Guard
/// must contact a remote server or it should be the same across calls *for a given contract
/// instance*.
pub enum Guard<ContractSelf> {
    /// Cache Variant should only be called one time per contract and the result saved
    Cache(fn(&ContractSelf, Context) -> Clause),
    /// Fresh Variant may be called repeatedly
    Fresh(fn(&ContractSelf, Context) -> Clause),
}

/// A List of Guards, for convenience
pub type GuardList<'a, T> = &'a [fn() -> Option<Guard<T>>];
