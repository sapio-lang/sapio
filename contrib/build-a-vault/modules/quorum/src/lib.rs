//! Sapio Studio module: Quorum.

pub use build_a_vault_blocks::Quorum;
#[cfg(target_arch = "wasm32")]
use sapio_wasm_plugin::{optional_logo, REGISTER};

#[cfg(target_arch = "wasm32")]
REGISTER![Quorum];
