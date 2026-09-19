//! Sapio Studio module: DelayedWallet.

pub use build_a_vault_blocks::DelayedWallet;
#[cfg(target_arch = "wasm32")]
use sapio_wasm_plugin::{optional_logo, REGISTER};

#[cfg(target_arch = "wasm32")]
REGISTER![DelayedWallet];
