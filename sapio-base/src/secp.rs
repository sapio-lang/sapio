// Copyright Judica, Inc 2026
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A process-wide shared secp256k1 context.
//!
//! Creating a `Secp256k1` context allocates and precomputes multiplication
//! tables, which dominates small operations such as Taproot tweaks, BIP32
//! derivation and signature checks when a fresh context is created per
//! call. Compiling one contract graph performs those operations once per
//! object, so per-call contexts multiply into real latency natively and
//! into metered fuel inside WASM guests.
//!
//! Non-test code shares this context instead of creating its own. The
//! shared context is never rerandomized, which is exactly the side-channel
//! profile of the unrandomized per-call contexts it replaces; callers with
//! stronger signing-hardening requirements can still construct and
//! rerandomize a private context.

use bitcoin::secp256k1::{All, Secp256k1};
use std::sync::OnceLock;

/// The shared all-capabilities context.
pub fn secp() -> &'static Secp256k1<All> {
    static SECP: OnceLock<Secp256k1<All>> = OnceLock::new();
    SECP.get_or_init(Secp256k1::new)
}
