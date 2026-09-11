//! JSON schema descriptions for upstream Bitcoin's human-readable serde format.
//!
//! These types describe contract interfaces; Bitcoin owns their actual parsing
//! and serialization. No cryptographic dependency needs a schema implementation.

use schemars::JsonSchema;

/// The JSON object serialized by [`bitcoin::Transaction`].
#[derive(JsonSchema)]
#[allow(dead_code)]
pub struct Transaction {
    version: i32,
    lock_time: u32,
    input: Vec<TxIn>,
    output: Vec<TxOut>,
}

/// A transaction input, with a displayed outpoint and hexadecimal witness items.
#[derive(JsonSchema)]
#[allow(dead_code)]
pub struct TxIn {
    previous_output: String,
    script_sig: String,
    sequence: u32,
    witness: Vec<String>,
}

/// A transaction output, with a satoshi amount and hexadecimal script.
#[derive(JsonSchema)]
#[allow(dead_code)]
pub struct TxOut {
    value: u64,
    script_pubkey: String,
}
