//! Public-key derivation used by Sapio's covenant policies.
//!
//! WASM guests delegate the expensive curve operations to the versioned,
//! metered host API. Native compilation uses the same Bitcoin library directly.

use bitcoin::util::bip32::{self, ChildNumber, ExtendedPubKey};
use bitcoin::XOnlyPublicKey;

/// Derive an exact non-hardened BIP32 path and return its x-only public key.
///
/// Invalid paths fail without selecting an alternate child. No secret key or
/// oracle runtime participates in public policy compilation.
pub(crate) fn derive_public_key(
    root: &ExtendedPubKey,
    path: &[ChildNumber],
) -> Result<XOnlyPublicKey, bip32::Error> {
    if path.len() > usize::from(u8::MAX - root.depth) {
        return Err(bip32::Error::InvalidDerivationPathFormat);
    }
    if path.iter().any(ChildNumber::is_hardened) {
        return Err(bip32::Error::CannotDeriveFromHardenedKey);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        root.derive_pub(&bitcoin::secp256k1::Secp256k1::verification_only(), &path)
            .map(|child| child.to_x_only_pub())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let path: Vec<u32> = path.iter().copied().map(u32::from).collect();
        let key = sapio_wasm::crypto::bip32_derive(&root.encode(), &path)
            .map_err(|_| bip32::Error::Secp256k1(bitcoin::secp256k1::Error::InvalidTweak))?;
        XOnlyPublicKey::from_slice(&key[1..]).map_err(bip32::Error::Secp256k1)
    }
}
