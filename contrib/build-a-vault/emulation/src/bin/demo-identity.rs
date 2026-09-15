//! Export the public identities used by the reproducible Studio tutorials.
//!
//! These public demo keys and addresses derive from deliberately public,
//! deterministic seeds. They are for local examples and tests; they provide
//! no private custody. The checked-in `demo-identity.json` contains only five
//! public keys, two regtest addresses, and the public emulation-oracle root.
//! Regenerate it by redirecting this binary's standard output to that file.

use bitcoin::bip32::{Xpriv, Xpub};
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{Address, Network};
use std::io::{self, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let secp = Secp256k1::new();
    let public_root = |seed: u8| -> Result<Xpub, bitcoin::bip32::Error> {
        let private = Xpriv::new_master(Network::Regtest, &[seed; 32])?;
        Ok(Xpub::from_priv(&secp, &private))
    };
    let keys = (1..=5)
        .map(|seed| public_root(seed).map(|root| root.to_x_only_pub()))
        .collect::<Result<Vec<_>, _>>()?;
    let addresses = [keys[3], keys[4]].map(|key| Address::p2tr(&secp, key, None, Network::Regtest));
    let identity = serde_json::json!({
        "keys": keys,
        "addresses": addresses,
        "oracle_xpub": public_root(42)?,
    });
    let mut output = io::stdout().lock();
    serde_json::to_writer_pretty(&mut output, &identity)?;
    writeln!(output)?;
    Ok(())
}
