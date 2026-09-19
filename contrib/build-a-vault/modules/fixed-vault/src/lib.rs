//! Sapio Studio module: FixedVault.

pub use build_a_vault_blocks::FixedVault;
#[cfg(target_arch = "wasm32")]
use sapio_wasm_plugin::{optional_logo, REGISTER};

#[cfg(target_arch = "wasm32")]
REGISTER![FixedVault];
