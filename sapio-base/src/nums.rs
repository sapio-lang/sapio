//! Public openings of script-only internal keys over the BIP341 NUMS point.

use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{Error, Scalar};
use bitcoin::XOnlyPublicKey;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The x-only BIP341 NUMS point H, with no known discrete logarithm.
pub const BIP341_NUMS: [u8; 32] = [
    0x50, 0x92, 0x9b, 0x74, 0xc1, 0xa0, 0x49, 0x54, 0xb7, 0x8b, 0x4b, 0x60, 0x35, 0xe9, 0x7a, 0x5e,
    0x07, 0x8a, 0x5a, 0x0f, 0x28, 0xec, 0x96, 0xd5, 0x47, 0xbf, 0xee, 0x9a, 0xce, 0x80, 0x3a, 0xc0,
];

/// An opening `internal_key = H + tweak*G` over the fixed BIP341 NUMS point.
///
/// Publishing the scalar establishes that a state commitment cannot introduce
/// a known key-path secret. The actual key must be rederived when validating a
/// serialized artifact; arbitrary public keys cannot assert this property.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NumsInternalKey {
    tweak: sha256::Hash,
}

impl NumsInternalKey {
    /// Validate a canonical scalar and the resulting finite internal point.
    pub fn new(tweak: [u8; 32]) -> Result<Self, Error> {
        let opening = Self {
            tweak: sha256::Hash::from_byte_array(tweak),
        };
        opening.internal_key()?;
        Ok(opening)
    }

    /// The public scalar opening, encoded as an unsigned big-endian integer.
    pub fn tweak(self) -> [u8; 32] {
        self.tweak.to_byte_array()
    }

    /// Rederive and validate the opened point, including decoded artifacts.
    pub fn internal_key(self) -> Result<XOnlyPublicKey, Error> {
        let tweak = Scalar::from_be_bytes(self.tweak()).map_err(|_| Error::InvalidTweak)?;
        XOnlyPublicKey::from_slice(&BIP341_NUMS)?
            .add_tweak(crate::secp::secp(), &tweak)
            .map(|(key, _)| key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_only_canonical_public_tweaks_of_the_fixed_nums_point() {
        let zero = NumsInternalKey::new([0; 32]).unwrap();
        assert_eq!(zero.internal_key().unwrap().serialize(), BIP341_NUMS);
        let commitment =
            NumsInternalKey::new(sha256::Hash::hash(b"state").to_byte_array()).unwrap();
        assert_ne!(
            commitment.internal_key().unwrap(),
            zero.internal_key().unwrap()
        );
        assert!(NumsInternalKey::new([255; 32]).is_err());
        let mut encoded = serde_json::to_value(zero).unwrap();
        encoded["tweak"] = serde_json::to_value(sha256::Hash::from_byte_array([255; 32])).unwrap();
        let decoded: NumsInternalKey = serde_json::from_value(encoded).unwrap();
        assert!(decoded.internal_key().is_err());
    }
}
