//! Shared execution limits and guest bindings for Sapio WASM modules.
#![cfg_attr(not(feature = "host"), no_std)]

/// Versioned native cryptography import namespace.
pub const CRYPTO_NAMESPACE: &str = "sapio_crypto_v1";
/// Largest input to one SHA256 import, in bytes.
pub const MAX_SHA256_BYTES: u32 = 1024 * 1024;
/// Maximum normal BIP32 children in one derivation.
pub const MAX_DERIVATION_CHILDREN: u32 = 255;

#[cfg(target_arch = "wasm32")]
pub mod crypto;

#[cfg(feature = "host")]
pub mod host;
