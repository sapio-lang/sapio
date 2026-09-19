//! Sapio Studio module: OpVault.

pub use build_a_vault_emulation::OpVault;
#[cfg(target_arch = "wasm32")]
use sapio_wasm_plugin::{optional_logo, REGISTER};

#[cfg(target_arch = "wasm32")]
REGISTER![OpVault];
