use bitcoin::consensus::deserialize;
use bitcoin::hashes::{hex::FromHex, sha256, Hash};
use bitcoin::Transaction;
use sapio_base::fragments::{template_hash, TemplateHashError};
use serde::Deserialize;

#[derive(Deserialize)]
struct Vector {
    spending_tx: String,
    input_index: u32,
    valid: bool,
    comment: String,
}

#[test]
fn template_hash_matches_all_official_bip446_cases() {
    let vectors: Vec<Vector> =
        serde_json::from_str(include_str!("data/bip446-basics.json")).unwrap();
    assert_eq!(vectors.len(), 19);
    for vector in vectors {
        let transaction: Transaction =
            deserialize(&Vec::<u8>::from_hex(&vector.spending_tx).unwrap()).unwrap();
        let mut script_witness = transaction.input[vector.input_index as usize]
            .witness
            .to_vec();
        if script_witness.last().unwrap().first() == Some(&0x50) {
            script_witness.pop();
        }
        let script = &script_witness[script_witness.len() - 2];
        assert_eq!(script.len(), 35);
        assert_eq!(script[0], 0x20);
        assert_eq!(&script[33..], &[0xce, 0x87]);
        let expected = sha256::Hash::from_slice(&script[1..33]).unwrap();
        let witness = &transaction.input[vector.input_index as usize].witness;
        let annex = if witness.len() >= 2 {
            witness.last().filter(|item| item.first() == Some(&0x50))
        } else {
            None
        };
        let actual = template_hash(&transaction, vector.input_index, annex).unwrap();
        assert_eq!(actual == expected, vector.valid, "{}", vector.comment);
    }
}

#[test]
fn template_hash_rejects_nonexistent_inputs_and_malformed_annexes() {
    let transaction = Transaction {
        version: 2,
        lock_time: 0,
        input: vec![bitcoin::TxIn::default()],
        output: vec![],
    };
    for index in [1, u32::MAX] {
        assert_eq!(
            template_hash(&transaction, index, None),
            Err(TemplateHashError::InputIndex)
        );
    }
    for annex in [&[][..], &[0][..], &[0x51, 0x50][..]] {
        assert_eq!(
            template_hash(&transaction, 0, Some(annex)),
            Err(TemplateHashError::Annex)
        );
    }
    assert_ne!(
        template_hash(&transaction, 0, None).unwrap(),
        template_hash(&transaction, 0, Some(&[0x50])).unwrap()
    );
}
