//! Sapio Studio module: Recovery.

pub use build_a_vault_blocks::Recovery;
#[cfg(target_arch = "wasm32")]
use sapio_wasm_plugin::{optional_logo, REGISTER};

#[cfg(target_arch = "wasm32")]
REGISTER![Recovery];
