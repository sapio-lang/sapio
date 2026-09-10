//! Resource-limited native WASM execution.

mod runtime;

pub use runtime::{new_store, store_with_fuel, INSTANCE_FUEL};
