//! Resource-limited native WASM execution.

mod crypto;
mod runtime;

pub use crypto::{
    add_crypto_imports, bind_crypto, CryptoEnvironment, BIP32_BASE_FUEL, BIP32_CHILD_FUEL,
    SCHNORR_VERIFY_FUEL, SHA256_BASE_FUEL, SHA256_BYTE_FUEL,
};
pub use runtime::{new_evaluator_store, new_store, store_with_fuel, INSTANCE_FUEL};
