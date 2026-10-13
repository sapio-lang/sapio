// Copyright Judica, Inc 2026
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The process-wide shared secp256k1 context.
//!
//! Creating a `Secp256k1` context allocates and precomputes multiplication
//! tables, which dominates small operations such as Taproot tweaks, BIP32
//! derivation and signature checks when a fresh context is created per
//! call. Compiling one contract graph performs those operations once per
//! object, so per-call contexts multiply into real latency natively and
//! into metered fuel inside WASM guests.
//!
//! Non-test code borrows the [`secp256k1` global context] instead of
//! creating its own; the pinned rust-miniscript fork shares the same
//! global, so descriptor lowering and contract compilation use one
//! context end to end. The global is rerandomized only when `rand-std`
//! is enabled on a native target — never inside WASM guests — which is
//! at least the side-channel profile of the unrandomized per-call
//! contexts it replaces; callers with stronger signing-hardening needs
//! can still construct and rerandomize a private context.
//!
//! [`secp256k1` global context]: bitcoin::secp256k1::SECP256K1

use bitcoin::secp256k1::{All, Secp256k1, SECP256K1};

/// The shared all-capabilities context.
pub fn secp() -> &'static Secp256k1<All> {
    SECP256K1
}
