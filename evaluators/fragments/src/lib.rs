//! Typed covenant fragments for the authenticated WASM-v2 transaction view.
//!
//! These helpers use the runtime's deterministic crypto imports. They do not
//! implement a Script VM or determine which covenant opcodes a chain enforces.
//! Compose fragments with ordinary Rust and propagate [`Failure`] with `?`.
#![no_std]
#![deny(missing_docs)]

#[cfg(test)]
extern crate std;

mod crypto;
mod view;
pub use view::Context;

/// Maximum encoded context and scratch space needed by these fragments.
pub const MAX_VIEW_BYTES: usize = 1_048_576;

/// A terminal evaluation failure, distinct from a false predicate result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Failure {
    /// A view, parameter, witness, or key has an invalid encoding.
    InvalidEncoding,
    /// The supplied scratch space cannot hold the required serialization.
    ScratchTooSmall,
    /// A native cryptographic import failed its ABI contract.
    Crypto,
    /// A nonempty CSFS signature is malformed or does not verify.
    Signature,
    /// A supplied additive-tweak proof does not open the actual spent key.
    TweakProof,
}

/// Exactly 32 bytes interpreted as a BIP340 x-only public key.
///
/// Point validity is checked when a nonempty signature or tweak proof uses it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct XOnlyKey([u8; 32]);

impl XOnlyKey {
    /// Read exactly one x-only key, rejecting truncated or trailing bytes.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, Failure> {
        bytes
            .try_into()
            .map(Self)
            .map_err(|_| Failure::InvalidEncoding)
    }

    /// Borrow the exact x-only key encoding.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// A BIP446 tagged template hash, including the selected input's annex.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TemplateHash(pub [u8; 32]);

impl TemplateHash {
    /// Borrow the 32-byte message used for a template authorization signature.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Verify a CSFS-style signature over an arbitrary byte message.
///
/// An empty signature returns `Ok(false)`. Every nonempty signature must be
/// exactly 64 bytes and verify under BIP340, or evaluation fails terminally.
/// No sighash byte is appended and the message is not implicitly prehashed.
pub fn check_sig_from_stack(
    message: &[u8],
    key: XOnlyKey,
    signature: &[u8],
) -> Result<bool, Failure> {
    if signature.is_empty() {
        return Ok(false);
    }
    let signature: &[u8; 64] = signature.try_into().map_err(|_| Failure::Signature)?;
    crypto::verify(message, key.as_bytes(), signature)?;
    Ok(true)
}

/// An additive opening `Q = lift_x(P) + t*G` of the spent output's key.
///
/// `P` is lifted with even Y. The witness commits to Q's full-point parity as
/// well as its x coordinate. This is an additive proof, not a claim that `t`
/// equals a particular TapTweak hash or BIP32 child derivation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KnownTweak {
    key: XOnlyKey,
    tweak: [u8; 32],
    parity: u8,
}

impl KnownTweak {
    /// Decode `P[32] || t[32] || parity[1]`, with big-endian scalar `t`.
    pub fn from_slice(proof: &[u8]) -> Result<Self, Failure> {
        if proof.len() != 65 || proof[64] > 1 {
            return Err(Failure::InvalidEncoding);
        }
        Ok(Self {
            key: XOnlyKey::from_slice(&proof[..32])?,
            tweak: proof[32..64]
                .try_into()
                .map_err(|_| Failure::InvalidEncoding)?,
            parity: proof[64],
        })
    }

    /// Return P only after its opening matches the authenticated spent key Q.
    pub fn authenticate(&self, context: &Context<'_>) -> Result<XOnlyKey, Failure> {
        crypto::tweak_check(
            self.key.as_bytes(),
            &self.tweak,
            context.output_key().as_bytes(),
            self.parity,
        )?;
        Ok(self.key)
    }
}
