use bitcoin::util::amount::Amount;
use sapio::contract::object::BadTxIndex;
use std::collections::HashMap;
use std::rc::Rc;
use std::str::FromStr;

use sapio::contract::*;
use sapio::*;

use bitcoin::util::bip32::*;
use emulator_connect::HDOracleEmulatorConnection;
use emulator_connect::*;

use bitcoin::secp256k1::Secp256k1;

use std::sync::Arc;
pub struct TestEmulation<T> {
    pub to_contract: T,
    pub amount: Amount,
    pub timeout: u32,
}

impl<T> TestEmulation<T>
where
    T: Compilable,
{
    then!(
        complete | s,
        ctx | {
            ctx.template()
                .add_output(s.amount, &s.to_contract, None)?
                .set_sequence(0, s.timeout)
                .into()
        }
    );
}

impl<T: Compilable + 'static> Contract for TestEmulation<T> {
    declare! {then, Self::complete}
    declare! {non updatable}
}

#[test]
fn test_connect() {
    let root =
        ExtendedPrivKey::new_master(bitcoin::network::constants::Network::Regtest, &[44u8; 32])
            .unwrap();
    let pk_root = ExtendedPubKey::from_private(&Secp256k1::new(), &root);
    let RT1 = Arc::new(tokio::runtime::Runtime::new().unwrap());
    let (shutdown, quit) = tokio::sync::oneshot::channel();
    {
        let RT = RT1.clone();
        std::thread::spawn(move || {
            RT.block_on(async {
                let oracle = HDOracleEmulator::new(root);
                let server = tokio::spawn(oracle.bind("127.0.0.1:8080"));
                quit.await;
                server.abort();
            })
        });
    };

    let contract_1 = TestEmulation {
        to_contract: Compiled::from_address(
            bitcoin::Address::from_str("bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh").unwrap(),
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
    let RT2 = Arc::new(tokio::runtime::Runtime::new().unwrap());
    let connecter = RT2.block_on(async {
        HDOracleEmulatorConnection::new("127.0.0.1:8080", pk_root, RT2.clone())
            .await
            .unwrap()
    });
    let rc_conn: Rc<dyn CTVEmulator> = Rc::new(connecter);
    let compiled = contract
        .compile(&Context::new(
            Amount::from_btc(1.0).unwrap(),
            Some(rc_conn.clone()),
        ))
        .unwrap();
    let _psbts = compiled.bind_psbt(
        bitcoin::OutPoint::default(),
        HashMap::new(),
        Rc::new(BadTxIndex::new()),
        rc_conn,
    );
    println!("HELLO");
    shutdown.send(());
    println!("HELLO");

    // TODO: Test PSBT result
}
