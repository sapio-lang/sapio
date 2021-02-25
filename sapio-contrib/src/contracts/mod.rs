//! A collection of contracts of varying quality and usefullness
use bitcoin::util::amount::CoinAmount;
use sapio_base::Clause;

use sapio::contract::*;
use sapio::*;
use schemars::*;
use serde::*;
use std::convert::TryInto;
pub mod basic_examples;
pub mod channel;
pub mod derivatives;
pub mod dynamic;
pub mod federated_sidechain;
pub mod hodl_chicken;
pub mod readme_contracts;
pub mod treepay;
pub mod undo_send;
pub mod vault;
