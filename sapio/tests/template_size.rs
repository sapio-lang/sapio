use bitcoin::consensus::serialize;
use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
use bitcoin::{Address, Amount, Network, Script, TxOut};
use sapio::contract::{Compiled, Context};
use sapio::template::Builder;
use sapio::util::extended_address::ExtendedAddress;
use sapio_base::covenant::LoweringPlan;
use sapio_base::miniscript::Descriptor;
use std::sync::Arc;

fn builder() -> Builder {
    Context::new(
        Network::Regtest,
        Amount::from_sat(1_000_000),
        LoweringPlan::Native,
        "size".try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    )
    .template()
}

fn destinations() -> Vec<Compiled> {
    let secp = Secp256k1::new();
    let pair = Keypair::from_secret_key(&secp, &SecretKey::from_slice(&[1; 32]).unwrap());
    let public = bitcoin::PublicKey::new(pair.public_key());
    let script = Script::new_op_return(&[1, 2, 3]);
    let mut values: Vec<_> = [
        Address::p2pkh(&public, Network::Regtest),
        Address::p2sh(&script, Network::Regtest).unwrap(),
        Address::p2wpkh(&public, Network::Regtest).unwrap(),
        Address::p2wsh(&script, Network::Regtest),
        Address::p2tr(&secp, pair.x_only_public_key().0, None, Network::Regtest),
    ]
    .into_iter()
    .map(|address| Compiled::from_address(address, Amount::ZERO))
    .collect();
    values.push(Compiled::from_op_return(&[4, 5, 6]).unwrap());
    values.push(Compiled::from_descriptor(
        Descriptor::new_wpkh(public).unwrap(),
        Amount::ZERO,
    ));
    values
}

#[test]
fn unsigned_size_counts_actual_scripts_for_every_destination_form() {
    let mut tx = builder();
    assert_eq!(tx.unsigned_tx_size(), 51);
    for destination in destinations() {
        tx = tx
            .add_output(Amount::from_sat(1), &destination, None)
            .unwrap();
        assert_eq!(tx.unsigned_tx_size(), serialize(&tx.get_tx()).len() as u64);
        assert_eq!(tx.unsigned_tx_size() * 4, tx.get_tx().weight() as u64);
    }
    let size = tx.unsigned_tx_size();
    let paying_fees = tx.add_fees(Amount::from_sat(100)).unwrap();
    assert_eq!(paying_fees.unsigned_tx_size(), size);
    assert_eq!(size, serialize(&paying_fees.get_tx()).len() as u64);
}

#[test]
fn output_script_compact_size_boundaries_are_counted_exactly() {
    for size in [0, 1, 252, 253, 65_535, 65_536] {
        let mut destination = destinations().remove(0);
        let script = Script::from(vec![0x61; size]);
        destination.address = ExtendedAddress::Unknown(script.clone());
        let tx = builder();
        let predicted = tx.unsigned_tx_size_with_output(&script);
        let tx = tx.add_output(Amount::ZERO, &destination, None).unwrap();
        assert_eq!(predicted, serialize(&tx.get_tx()).len() as u64);
        assert_eq!(tx.unsigned_tx_size(), predicted);
    }
}

#[test]
fn input_and_output_count_boundaries_include_their_compact_size_growth() {
    let destination = destinations().remove(0);
    let script = Script::from(&destination.address);
    let mut tx = builder();
    for inputs in 1..=253 {
        if inputs > 1 {
            tx = tx.add_sequence();
        }
        if [1, 252, 253].contains(&inputs) {
            assert_eq!(tx.unsigned_tx_size(), serialize(&tx.get_tx()).len() as u64);
        }
    }
    for outputs in 1..=253 {
        let predicted = tx.unsigned_tx_size_with_output(&script);
        tx = tx.add_output(Amount::ZERO, &destination, None).unwrap();
        if [1, 252, 253].contains(&outputs) {
            assert_eq!(tx.unsigned_tx_size(), serialize(&tx.get_tx()).len() as u64);
            assert_eq!(tx.unsigned_tx_size(), predicted);
        }
    }
    // The look-ahead is independent of the eventual output value.
    let predicted = tx.unsigned_tx_size_with_output(&script);
    let mut transaction = tx.get_tx();
    transaction.output.push(TxOut {
        value: u64::MAX,
        script_pubkey: script,
    });
    assert_eq!(predicted, serialize(&transaction).len() as u64);
}
