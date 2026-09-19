//! Semantic identities for public values in module schemas.
//!
//! JSON Schema describes representation and local constraints. These identities
//! additionally distinguish values with the same JSON representation, such as a
//! key and an address, or a block delay and a satoshi amount. They are interface
//! declarations, not attestations that a producer implements a sound policy.

use bitcoin::{address::NetworkUnchecked, bip32::Xpub, Address, XOnlyPublicKey};
use schemars::{JsonSchema, Schema, SchemaGenerator};

fn named<T: JsonSchema>(generator: &mut SchemaGenerator, id: &str, title: &str) -> Schema {
    let mut schema = T::json_schema(generator);
    schema.insert("x-sapio-type".into(), id.into());
    schema.insert("title".into(), title.into());
    schema
}

/// Schema for a public BIP340 signing key.
pub fn x_only_public_key(generator: &mut SchemaGenerator) -> Schema {
    named::<XOnlyPublicKey>(generator, "bitcoin.x-only-public-key", "Public key")
}

/// A list whose elements retain their public-key identity.
pub fn public_keys(generator: &mut SchemaGenerator) -> Schema {
    schemars::json_schema!({"type": "array", "items": x_only_public_key(generator)})
}

/// Schema for an address; consumers still check its compilation network.
pub fn address(generator: &mut SchemaGenerator) -> Schema {
    named::<Address<NetworkUnchecked>>(generator, "bitcoin.address", "Bitcoin address")
}

/// Schema for a BIP32 public extended key.
pub fn xpub(generator: &mut SchemaGenerator) -> Schema {
    named::<Xpub>(generator, "bitcoin.xpub", "Extended public key")
}

/// Schema for an integer number of satoshis.
pub fn satoshis(generator: &mut SchemaGenerator) -> Schema {
    named::<u64>(generator, "bitcoin.satoshis", "Satoshis")
}

/// Schema for a positive block-denominated relative delay.
pub fn relative_blocks(generator: &mut SchemaGenerator) -> Schema {
    let mut schema = named::<u16>(generator, "bitcoin.relative-blocks", "Block count");
    schema.insert("minimum".into(), 1.into());
    schema
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_value_schemas_keep_wire_constraints_and_distinct_units() {
        let mut generator = schemars::generate::SchemaSettings::draft07().into_generator();
        let key = x_only_public_key(&mut generator);
        let keys = public_keys(&mut generator);
        assert_eq!(keys.as_value()["items"], *key.as_value());
        assert_eq!(key.as_value()["x-sapio-type"], "bitcoin.x-only-public-key");
        assert_eq!(key.as_value()["type"], "string");
        let sats = satoshis(&mut generator);
        let blocks = relative_blocks(&mut generator);
        assert_eq!(sats.as_value()["type"], blocks.as_value()["type"]);
        assert_ne!(
            sats.as_value()["x-sapio-type"],
            blocks.as_value()["x-sapio-type"]
        );
        assert_eq!(blocks.as_value()["minimum"], 1);
        assert_eq!(blocks.as_value()["maximum"], u16::MAX);
    }
}
