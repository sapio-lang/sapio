// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use bitcoin::secp256k1::Secp256k1;
use bitcoin::util::amount::Amount;
use bitcoin::util::bip32::*;
use bitcoin::TxOut;
use emulator_connect::connections::hd::HDOracleEmulatorConnection;
use emulator_connect::servers::hd::HDOracleEmulator;
use emulator_connect::*;
use miniscript::psbt::PsbtExt;
use sapio::contract::*;
use sapio::*;
use sapio_base::effects::EffectPath;
use sapio_base::timelocks::RelTime;
use sapio_base::txindex::{TxIndex, TxIndexLogger};
use std::collections::BTreeMap;
use std::convert::TryFrom;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;

pub struct TestEmulation<T> {
    pub to_contract: T,
    pub amount: Amount,
    pub timeout: u16,
}

impl<T: 'static> TestEmulation<T>
where
    T: Compilable,
{
    #[then]
    fn complete(self, ctx: Context) {
        ctx.template()
            .add_output(self.amount, &self.to_contract, None)?
            .set_sequence(0, RelTime::from(self.timeout).into())?
            .into()
    }
}

impl<T: Compilable + 'static> Contract for TestEmulation<T> {
    declare! {then, Self::complete}
    declare! {non updatable}
}

#[tokio::test(flavor = "multi_thread")]
async fn compiles_signs_and_finalizes_a_two_step_contract() {
    let secp = Secp256k1::new();
    let root =
        ExtendedPrivKey::new_master(bitcoin::network::constants::Network::Regtest, &[44u8; 32])
            .unwrap();
    let pk_root = ExtendedPubKey::from_priv(&secp, &root);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(HDOracleEmulator::new(root).serve(listener));

    let contract_1 = TestEmulation {
        to_contract: Compiled::from_address(
            bitcoin::Address::from_str(
                "tb1pnt49mgrp6djyzj7ttldle9lhnhav9hh7pcaqmv9yqpfrwk4yzvasd8wc37",
            )
            .unwrap(),
            None,
        ),
        amount: Amount::from_btc(1.0).unwrap(),
        timeout: 6,
    };
    let contract = TestEmulation {
        to_contract: contract_1,
        amount: Amount::from_btc(1.0).unwrap(),
        timeout: 4,
    };
    let connecter =
        HDOracleEmulatorConnection::new(address, pk_root, None, Arc::new(Secp256k1::new()))
            .await
            .unwrap();
    let rc_conn: Arc<dyn CTVEmulator> = Arc::new(connecter);
    let compiled = contract
        .compile(Context::new(
            bitcoin::Network::Regtest,
            Amount::from_btc(1.0).unwrap(),
            rc_conn.clone(),
            EffectPath::try_from("integration_test").unwrap(),
            Arc::new(Default::default()),
            None,
        ))
        .unwrap();
    let txindex: Rc<dyn TxIndex> = Rc::new(TxIndexLogger::new());
    let tx = bitcoin::Transaction {
        version: 2,
        lock_time: 0,
        input: vec![bitcoin::TxIn::default()],
        output: vec![TxOut {
            value: Amount::from_btc(1.0).unwrap().as_sat(),
            script_pubkey: compiled.address.clone().into(),
        }],
    };
    let fake_txid = txindex.add_tx(std::sync::Arc::new(tx)).unwrap();
    let psbts = compiled.bind_psbt(
        bitcoin::OutPoint::new(fake_txid, 0),
        BTreeMap::new(),
        txindex,
        rc_conn.as_ref(),
    );
    use bitcoin::psbt::PartiallySignedTransaction;
    use sapio::contract::abi::studio::SapioStudioFormat;

    let mut sequences = Vec::new();
    for sso in psbts.unwrap().program.values() {
        for tx in &sso.txs {
            let SapioStudioFormat::LinkedPSBT { psbt, .. } = tx;
            let mut psbt = PartiallySignedTransaction::from_str(psbt).unwrap();
            assert_eq!(psbt.inputs.len(), 1);
            let original_txid = psbt.unsigned_tx.txid();
            let mut tampered = psbt.clone();
            tampered.unsigned_tx.output[0].value -= 1;
            assert!(tampered.finalize_mut(&secp).is_err());
            psbt.finalize_mut(&secp).unwrap();
            let finalized = psbt.extract_tx();
            assert_eq!(finalized.txid(), original_txid);
            assert!(!finalized.input[0].witness.is_empty());
            assert_eq!(finalized.output[0].value, 100_000_000);
            sequences.push(finalized.input[0].sequence);
        }
    }
    sequences.sort_unstable();
    // BIP-68 time-based sequences retain the type flag and 512-second units.
    assert_eq!(sequences, [(1 << 22) | 4, (1 << 22) | 6]);
    drop(rc_conn);
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
