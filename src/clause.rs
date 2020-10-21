use bitcoin::hashes::hex::FromHex;
use bitcoin::hashes::{hash160, ripemd160, sha256, sha256d};
use lazy_static::lazy_static;

/// Concrete Instantiation of Miniscript Policy. Because we need to be able to generate exact
/// transactions, we only work with `bitcoin::PublicKey` types.
pub type Clause = miniscript::policy::concrete::Policy<bitcoin::PublicKey>;

