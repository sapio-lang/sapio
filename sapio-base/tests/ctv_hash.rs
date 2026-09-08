use bitcoin::consensus::deserialize;
use bitcoin::hashes::{hex::FromHex, sha256};
use bitcoin::Transaction;
use sapio_base::CTVHash;
use serde::Deserialize;

#[derive(Deserialize)]
struct Vector {
    hex_tx: String,
    spend_index: Vec<u32>,
    result: Vec<String>,
}

#[test]
fn matches_bip119_hash_vectors() {
    let entries: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/ctvhash.json")).unwrap();
    let mut transactions = 0;
    let mut hashes = 0;
    for entry in entries {
        // The official file includes a format header and a trailing comment.
        if entry.is_string() {
            continue;
        }
        let vector: Vector = serde_json::from_value(entry).unwrap();
        let tx: Transaction = deserialize(&Vec::<u8>::from_hex(&vector.hex_tx).unwrap()).unwrap();
        assert_eq!(vector.spend_index.len(), vector.result.len());
        for (index, expected) in vector.spend_index.into_iter().zip(vector.result) {
            assert_eq!(
                tx.get_ctv_hash(index),
                sha256::Hash::from_hex(&expected).unwrap(),
                "BIP-119 hash for input {index} of {}",
                vector.hex_tx,
            );
            hashes += 1;
        }
        transactions += 1;
    }
    assert_eq!(transactions, 100);
    assert_eq!(hashes, 400);
}
