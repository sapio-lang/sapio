//! A disposable runner for the public eltoo contract in `sapio-contrib`.
//!
//! Contract source lives in [`sapio_contrib::contracts::eltoo`]. This module
//! assembles regtest compilation, authenticated spending requests and recovery.
//! The fixture's one joint key simulates two-party authorization, not MuSig2.

pub mod fixture;
pub mod recovery;
pub mod runner;
