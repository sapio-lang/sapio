#![deny(missing_docs)]

//! TreePay exposed through the exact batching module interface.

#[cfg(target_arch = "wasm32")]
use batching_trait::Versions;
#[cfg(target_arch = "wasm32")]
use sapio_treepay_contract::TreePay as TreePayBatching;
#[cfg(target_arch = "wasm32")]
use sapio_wasm_plugin::{optional_logo, REGISTER};

#[cfg(target_arch = "wasm32")]
REGISTER![[TreePayBatching, Versions]];
