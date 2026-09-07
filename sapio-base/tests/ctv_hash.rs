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
    let vectors: Vec<Vector> = serde_json::from_str(include_str!("data/ctvhash.json")).unwrap();
    for vector in vectors {
        let tx: Transaction = deserialize(&Vec::<u8>::from_hex(&vector.hex_tx).unwrap()).unwrap();
        assert_eq!(vector.spend_index.len(), vector.result.len());
        for (index, expected) in vector.spend_index.into_iter().zip(vector.result) {
            assert_eq!(
                tx.get_ctv_hash(index),
                sha256::Hash::from_hex(&expected).unwrap(),
                "BIP-119 hash for input {index} of {}",
                vector.hex_tx,
            );
        }
    }
}
