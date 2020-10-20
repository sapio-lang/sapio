use bitcoin::hashes::{hash160, ripemd160, sha256, sha256d};
use ::miniscript::*;
use std::collections::HashMap;
use std::default::Default;

pub mod clause;
#[macro_use]
pub mod contract;
pub mod txn;
pub mod util;
use clause::Clause;
use txn::Template as TransactionTemplate;
use util::amountrange::AmountRange;
