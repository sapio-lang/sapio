//! Sapio Studio module: Signer.

pub use build_a_vault_blocks::Signer;
#[cfg(target_arch = "wasm32")]
use sapio_wasm_plugin::{optional_logo, REGISTER};

#[cfg(target_arch = "wasm32")]
REGISTER![Signer];
