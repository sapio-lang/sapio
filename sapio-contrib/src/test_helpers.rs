use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
use bitcoin::{Amount, Network, XOnlyPublicKey};
use sapio::contract::Context;
use sapio_base::covenant::LoweringPlan;
use sapio_base::effects::EffectPath;
use std::sync::Arc;

pub(crate) fn context(sats: u64) -> Context {
    Context::new(
        Network::Regtest,
        Amount::from_sat(sats),
        LoweringPlan::Native,
        EffectPath::try_from("example").unwrap(),
        Arc::new(Default::default()),
        None,
    )
}

pub(crate) fn key(byte: u8) -> XOnlyPublicKey {
    Keypair::from_secret_key(
        &Secp256k1::new(),
        &SecretKey::from_slice(&[byte; 32]).unwrap(),
    )
    .x_only_public_key()
    .0
}

pub(crate) fn address(byte: u8) -> bitcoin::Address {
    bitcoin::Address::p2tr(&Secp256k1::new(), key(byte), None, Network::Regtest)
}
