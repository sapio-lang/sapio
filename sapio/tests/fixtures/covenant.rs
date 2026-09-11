#![allow(dead_code)]

use bitcoin::bip32::{Xpriv, Xpub};
use bitcoin::hashes::sha256;
use bitcoin::secp256k1::{Keypair, Secp256k1};
use bitcoin::{Network, XOnlyPublicKey};
use sapio_base::covenant::{hash_to_child_vec, Ctv, LoweringPlan};
use sapio_base::Clause;

pub fn private_root(seed: u8) -> Xpriv {
    Xpriv::new_master(Network::Testnet, &[seed; 32]).unwrap()
}

pub fn public_root(seed: u8) -> Xpub {
    Xpub::from_priv(&Secp256k1::new(), &private_root(seed))
}

pub fn plan(seed: u8) -> LoweringPlan {
    LoweringPlan::CtvEmulation {
        signers: vec![public_root(seed)],
        threshold: 1,
    }
}

pub fn keypair(seed: u8, hash: sha256::Hash) -> Keypair {
    let secp = Secp256k1::new();
    let derived = private_root(seed)
        .derive_priv(&secp, &hash_to_child_vec(hash))
        .unwrap();
    let keypair = Keypair::from_secret_key(&secp, &derived.private_key);
    assert_eq!(
        plan(seed).lower_ctv(Ctv(hash)).unwrap(),
        Clause::Key(keypair.x_only_public_key().0)
    );
    keypair
}

pub fn key(seed: u8, hash: sha256::Hash) -> XOnlyPublicKey {
    keypair(seed, hash).x_only_public_key().0
}
