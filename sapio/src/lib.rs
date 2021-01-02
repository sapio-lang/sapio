#![cfg_attr(feature = "nightly", feature(associated_type_defaults))]
extern crate serde;

pub mod core;
pub use crate::core::*;

#[cfg(feature = "examples")]
pub mod example_contracts;
