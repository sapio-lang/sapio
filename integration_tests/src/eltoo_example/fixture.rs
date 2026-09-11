//! Disposable keys and synthetic funding for the eltoo runner and tests.

use super::*;
use bitcoin::bip32::Xpriv;
use bitcoin::secp256k1::SecretKey;

fn key(seed: u8) -> Keypair {
    Keypair::from_secret_key(
        &Secp256k1::new(),
        &SecretKey::from_slice(&[seed; 32]).unwrap(),
    )
}
pub fn joint_key() -> Keypair {
    key(101)
}
pub fn sponsor_key() -> Keypair {
    key(103)
}
pub fn oracle_key() -> Xpriv {
    Xpriv::new_master(Network::Testnet, &[102; 32]).unwrap()
}
pub fn terms() -> Terms {
    Terms::new(
        joint_key().x_only_public_key().0,
        Xpub::from_priv(&Secp256k1::new(), &oracle_key()),
        100_000,
        6,
        MAX_STATE,
        crate::program_example::recipient(104),
        crate::program_example::recipient(105),
    )
    .unwrap()
}
fn outpoint(tag: u8) -> OutPoint {
    OutPoint::new(
        bitcoin::Txid::from_byte_array(sha256::Hash::hash(&[tag]).to_byte_array()),
        0,
    )
}
pub fn coin(channel: &Channel, tag: u8) -> Coin {
    Coin {
        outpoint: outpoint(tag),
        txout: TxOut {
            value: bitcoin::Amount::from_sat(channel.terms.capacity),
            script_pubkey: (&channel.compile().unwrap().address).into(),
        },
    }
}
pub fn sponsor(value: u64, tag: u8) -> Sponsor {
    let internal_key = sponsor_key().x_only_public_key().0;
    Sponsor {
        coin: Coin {
            outpoint: outpoint(tag),
            txout: TxOut {
                value: Amount::from_sat(value),
                script_pubkey: Address::p2tr(
                    &Secp256k1::new(),
                    internal_key,
                    None,
                    Network::Regtest,
                )
                .script_pubkey(),
            },
        },
        internal_key,
    }
}
