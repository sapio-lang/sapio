use super::*;
use bitcoin::consensus::{deserialize, encode::serialize_hex, serialize};
use bitcoin::psbt::raw::Key;
use bitcoin::secp256k1::{Message, Secp256k1, SecretKey};
use bitcoin::{
    Address, EcdsaSig, EcdsaSighashType, Network, PublicKey, Script, Transaction, TxIn, TxOut,
};
use sapio::contract::abi::studio::SapioStudioFormat;
use std::str::FromStr;

fn contract() -> Compiled {
    Compiled::from_address(
        Address::from_str("bcrt1qumrrqgt7e3a7damzm8x97m6sjs20u8hjw2hcjj").unwrap(),
        bitcoin::Amount::ZERO,
    )
}

fn funding_psbt(compiled: &Compiled) -> PartiallySignedTransaction {
    let tx = Transaction {
        version: 2,
        lock_time: 0,
        input: vec![TxIn::default()],
        output: vec![
            TxOut {
                value: 500,
                script_pubkey: Script::new(),
            },
            TxOut {
                value: 1_000,
                script_pubkey: (&compiled.address).into(),
            },
        ],
    };
    let mut psbt = PartiallySignedTransaction::from_unsigned_tx(tx).unwrap();
    let unknown = |key| Key {
        type_value: 0xfa,
        key: vec![key],
    };
    psbt.unknown.insert(unknown(1), vec![2]);
    psbt.inputs[0].unknown.insert(unknown(3), vec![4]);
    psbt.outputs[1].unknown.insert(unknown(5), vec![6]);
    psbt
}

fn request(compiled: Compiled, psbt: &[u8]) -> Bind {
    Bind {
        client_url: "http://127.0.0.1:1".into(),
        client_auth: rpc::Auth::None,
        use_base64: true,
        use_mock: false,
        outpoint: None,
        use_txn: Some(base64::encode(psbt)),
        compiled,
        ordinals_info: None,
    }
}

fn assert_preserved_funding(
    bound: &Program,
    compiled: &Compiled,
    expected: &PartiallySignedTransaction,
) {
    let expected_tx = expected.clone().extract_tx();
    assert_eq!(
        bound.program.get(&compiled.root_path).unwrap().out,
        OutPoint::new(expected_tx.txid(), 1)
    );
    let funding = bound
        .program
        .values()
        .flat_map(|object| &object.txs)
        .find(|entry| {
            let SapioStudioFormat::LinkedPSBT { metadata, .. } = entry;
            metadata.label.as_deref() == Some("funding")
        })
        .unwrap();
    let SapioStudioFormat::LinkedPSBT { psbt, hex, .. } = funding;
    let restored: PartiallySignedTransaction = deserialize(&base64::decode(psbt).unwrap()).unwrap();
    assert_eq!(restored, *expected);
    assert_eq!(*hex, serialize_hex(&expected_tx));
}

#[tokio::test]
async fn supplied_funding_keeps_partial_signatures_and_all_psbt_maps() {
    let compiled = contract();
    let mut psbt = funding_psbt(&compiled);
    let secp = Secp256k1::new();
    let secret = SecretKey::from_slice(&[1; 32]).unwrap();
    let key = PublicKey::new(bitcoin::secp256k1::PublicKey::from_secret_key(
        &secp, &secret,
    ));
    let sig = EcdsaSig {
        sig: secp.sign_ecdsa(&Message::from_digest_slice(&[2; 32]).unwrap(), &secret),
        hash_ty: EcdsaSighashType::All,
    };
    psbt.inputs[0].partial_sigs.insert(key, sig);
    let bound = request(compiled.clone(), &serialize(&psbt))
        .call(Network::Regtest, Arc::new(CTVAvailable))
        .await
        .unwrap();
    assert_preserved_funding(&bound, &compiled, &psbt);
}

#[tokio::test]
async fn finalized_legacy_funding_keeps_its_psbt_and_binds_the_extracted_txid() {
    let compiled = contract();
    let mut psbt = funding_psbt(&compiled);
    psbt.inputs[0].final_script_sig = Some(Script::from(vec![1, 0x42]));
    assert_ne!(psbt.unsigned_tx.txid(), psbt.clone().extract_tx().txid());
    let bound = request(compiled.clone(), &serialize(&psbt))
        .call(Network::Regtest, Arc::new(CTVAvailable))
        .await
        .unwrap();
    assert_preserved_funding(&bound, &compiled, &psbt);
}

#[tokio::test]
async fn zero_input_funding_psbt_returns_a_validation_error() {
    let compiled = contract();
    let mut psbt = funding_psbt(&compiled);
    psbt.unsigned_tx.input.clear();
    psbt.inputs.clear();
    let error = request(compiled, &serialize(&psbt))
        .call(Network::Regtest, Arc::new(CTVAvailable))
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<sapio_psbt::PSBTValidationError>(),
        Some(sapio_psbt::PSBTValidationError::NoInputs)
    ));
}

#[tokio::test]
async fn supplied_funding_rejects_trailing_psbt_bytes() {
    let compiled = contract();
    let mut bytes = serialize(&funding_psbt(&compiled));
    bytes.push(0);
    assert!(request(compiled, &bytes)
        .call(Network::Regtest, Arc::new(CTVAvailable))
        .await
        .is_err());
}

#[test]
fn funding_output_uses_the_contract_script_and_rejects_missing_outputs() {
    let compiled = contract();
    let script = bitcoin::Script::from(&compiled.address);
    let mut psbt = funding_psbt(&compiled);
    assert_eq!(funding_output(&psbt, &script).unwrap(), 1);
    psbt.unsigned_tx.output.swap(0, 1);
    psbt.outputs.swap(0, 1);
    assert_eq!(funding_output(&psbt, &script).unwrap(), 0);

    psbt.unsigned_tx.output[0].script_pubkey = Script::new();
    assert!(funding_output(&psbt, &script)
        .unwrap_err()
        .is::<RequestError>());
    psbt.unsigned_tx.output.clear();
    psbt.outputs.clear();
    assert!(funding_output(&psbt, &script)
        .unwrap_err()
        .is::<RequestError>());
}

#[test]
fn funding_output_validates_psbt_maps_before_extraction() {
    use sapio_psbt::PSBTValidationError;

    let compiled = contract();
    let script = bitcoin::Script::from(&compiled.address);
    let mut psbt = funding_psbt(&compiled);
    psbt.inputs.clear();
    assert_eq!(
        funding_output(&psbt, &script)
            .unwrap_err()
            .downcast_ref::<PSBTValidationError>(),
        Some(&PSBTValidationError::InputMapCount {
            transaction: 1,
            maps: 0,
        })
    );

    let mut psbt = funding_psbt(&compiled);
    psbt.outputs.pop();
    assert_eq!(
        funding_output(&psbt, &script)
            .unwrap_err()
            .downcast_ref::<PSBTValidationError>(),
        Some(&PSBTValidationError::OutputMapCount {
            transaction: 2,
            maps: 1,
        })
    );
}

#[test]
fn rpc_funding_must_match_the_requested_txid_and_output_index() {
    let tx = funding_psbt(&contract()).extract_tx();
    let requested = OutPoint::new(tx.txid(), 1);
    validate_funding_outpoint(&tx, requested).unwrap();
    assert!(matches!(
        validate_funding_outpoint(&tx, OutPoint::new(tx.txid(), 2)),
        Err(TxIndexError::IndexTooHigh(2))
    ));

    let mut wrong = tx;
    wrong.output[1].value += 1;
    for vout in [1, 2] {
        assert!(matches!(
            validate_funding_outpoint(&wrong, OutPoint { vout, ..requested }),
            Err(TxIndexError::TxidMismatch { expected, actual })
                if expected == requested.txid && actual == wrong.txid()
        ));
    }
}

#[tokio::test]
async fn mock_funding_still_includes_a_funding_psbt() {
    let compiled = contract();
    let mut bind = request(compiled.clone(), &serialize(&funding_psbt(&compiled)));
    bind.use_txn = None;
    bind.use_mock = true;
    let bound = bind
        .call(Network::Regtest, Arc::new(CTVAvailable))
        .await
        .unwrap();
    let funding = bound
        .program
        .values()
        .flat_map(|object| &object.txs)
        .next()
        .unwrap();
    let SapioStudioFormat::LinkedPSBT { psbt, .. } = funding;
    let psbt: PartiallySignedTransaction = deserialize(&base64::decode(psbt).unwrap()).unwrap();
    sapio_psbt::validate_psbt(&psbt).unwrap();
    assert_eq!(
        psbt.unsigned_tx.input[0].previous_output,
        create_mock_output()
    );
    assert_eq!(psbt.unsigned_tx.output.len(), 1);
    assert_eq!(
        psbt.unsigned_tx.output[0].script_pubkey,
        bitcoin::Script::from(&compiled.address)
    );
    assert_eq!(
        bound.program.get(&compiled.root_path).unwrap().out,
        OutPoint::new(psbt.unsigned_tx.txid(), 0)
    );
}

#[tokio::test]
async fn funding_entry_cannot_overwrite_a_contract_named_funding() {
    for root in ["funding", "funding/@funding"] {
        let mut compiled = contract();
        compiled.root_path = SArc(Arc::new(root.try_into().unwrap()));
        let psbt = funding_psbt(&compiled);
        let bound = request(compiled.clone(), &serialize(&psbt))
            .call(Network::Regtest, Arc::new(CTVAvailable))
            .await
            .unwrap();
        assert_eq!(bound.program.len(), 2);
        assert_preserved_funding(&bound, &compiled, &psbt);
        let contract_node = bound.program.get(&compiled.root_path).unwrap();
        assert_eq!(
            contract_node.source_path.as_ref(),
            Some(&compiled.root_path)
        );
        let funding_path = SArc(Arc::new(format!("{root}/@funding").try_into().unwrap()));
        let funding = bound.program.get(&funding_path).unwrap();
        assert!(funding.source_path.is_none());
        let restored: Program =
            serde_json::from_value(serde_json::to_value(&bound).unwrap()).unwrap();
        assert!(restored
            .program
            .get(&funding_path)
            .unwrap()
            .source_path
            .is_none());
    }
}
