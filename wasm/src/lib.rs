//! Shared execution limits and guest bindings for Sapio WASM modules.
#![cfg_attr(not(feature = "host"), no_std)]

#[cfg(feature = "host")]
pub mod host;
