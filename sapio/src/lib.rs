//! The Sapio Compiler Core Crate. Sapio is used to create multi-transaction Bitcoin Smart Contracts.
#![cfg_attr(feature = "nightly", feature(associated_type_defaults))]
#![deny(missing_docs)]
extern crate serde;

#[macro_use]
pub mod contract;
pub mod template;
pub mod util;
