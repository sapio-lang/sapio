use bitcoin::hashes::hex::FromHex;
use bitcoin::hashes::{hash160, ripemd160, sha256, sha256d};
use lazy_static::lazy_static;
pub type Clause = miniscript::policy::concrete::Policy<bitcoin::PublicKey>;

lazy_static! {
    pub static ref UNSATISIFIABLE: Clause = Clause::Sha256(
        sha256::Hash::from_hex("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF")
            .unwrap()
    );
    pub static ref SATISIFIABLE: Clause = Clause::Sha256(
        sha256::Hash::from_hex("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
            .unwrap()
    );
}
