//! Sapio Studio module: EmulationOracle.

pub use build_a_vault_emulation::EmulationOracle;
#[cfg(target_arch = "wasm32")]
use sapio_wasm_plugin::{optional_logo, REGISTER};

#[cfg(target_arch = "wasm32")]
REGISTER![EmulationOracle];
