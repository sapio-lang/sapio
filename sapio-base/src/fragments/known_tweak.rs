use super::known_tweak_witness;
use crate::program::{program_derivation_path, EmulatedProgram};
use bitcoin::bip32;
use bitcoin::hashes::Hash;
use bitcoin::secp256k1::{schnorr::Signature, Parity, Scalar, Secp256k1, SecretKey};
use bitcoin::taproot::{TapNodeHash, TapTweakHash};
use bitcoin::XOnlyPublicKey;
use std::fmt;

/// Public proof that an emulation root opens its program's Taproot output key.
///
/// The opening is `Q = lift_x(P) + t*G`, where P is the x-only root key. Both
/// BIP32 child derivation and the final TapTweak use public information only.
/// The signature attached by [`Self::witness`] must be made under P.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KnownTweakProof {
    key: XOnlyPublicKey,
    tweak: Scalar,
    parity: Parity,
}

impl KnownTweakProof {
    /// Open the output obtained by applying TapTweak to this program's key.
    ///
    /// Derivation follows the exact committed program path without skipping
    /// invalid children. Call [`Self::check_output`] against the actual spent
    /// output key to check that the supplied Merkle root describes that coin.
    pub fn for_program(
        program: &EmulatedProgram,
        merkle_root: Option<TapNodeHash>,
    ) -> Result<Self, KnownTweakError> {
        let secp = Secp256k1::verification_only();
        let root = *program.root();
        let (key, root_parity) = root.public_key.x_only_public_key();
        let mut child = root;
        let mut cumulative = Scalar::ZERO;
        for index in program_derivation_path(program.instance().id()) {
            let (tweak, _) = child
                .ckd_pub_tweak(index)
                .map_err(KnownTweakError::Derivation)?;
            cumulative = add(cumulative, Scalar::from(tweak));
            child = child
                .ckd_pub(&secp, index)
                .map_err(KnownTweakError::Derivation)?;
        }
        let (internal, child_parity) = child.public_key.x_only_public_key();
        let tap = Scalar::from_be_bytes(
            TapTweakHash::from_key_and_tweak(internal, merkle_root).to_byte_array(),
        )
        .map_err(|_| KnownTweakError::InvalidTweak)?;
        let (output_key, output_parity) = internal
            .add_tweak(&secp, &tap)
            .map_err(|_| KnownTweakError::InvalidTweak)?;

        // Full child = root + C*G. Account for both x-only normalizations,
        // choosing Q or -Q so the lifted root P has coefficient +1.
        let output_sign = root_parity ^ child_parity;
        let proof = Self {
            key,
            tweak: add(
                with_sign(cumulative, root_parity),
                with_sign(tap, output_sign),
            ),
            parity: output_parity ^ output_sign,
        };
        proof.check_output(output_key)?;
        Ok(proof)
    }

    /// The untweaked x-only root key that must authorize the template.
    pub fn key(&self) -> XOnlyPublicKey {
        self.key
    }

    /// The public additive opening scalar, including the Taproot tweak.
    pub fn tweak(&self) -> Scalar {
        self.tweak
    }

    /// Parity of the output representative opened by the lifted root key.
    pub fn parity(&self) -> Parity {
        self.parity
    }

    /// Verify the opening against the actual spent output's x-only key.
    pub fn check_output(&self, output_key: XOnlyPublicKey) -> Result<(), KnownTweakError> {
        if self.key.tweak_add_check(
            &Secp256k1::verification_only(),
            &output_key,
            self.parity,
            self.tweak,
        ) {
            Ok(())
        } else {
            Err(KnownTweakError::OutputKeyMismatch)
        }
    }

    /// Attach a raw BIP340 template signature to this public opening.
    pub fn witness(&self, signature: &Signature) -> Vec<u8> {
        known_tweak_witness(self.key, self.tweak, self.parity, Some(signature))
    }
}

/// A public derivation or output opening could not be established.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KnownTweakError {
    /// The exact BIP32 program path contains an invalid child derivation.
    Derivation(bip32::Error),
    /// The TapTweak scalar is out of range or produces the point at infinity.
    InvalidTweak,
    /// The public opening does not match the supplied output key.
    OutputKeyMismatch,
}

impl fmt::Display for KnownTweakError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Derivation(error) => write!(formatter, "program key derivation: {error}"),
            Self::InvalidTweak => formatter.write_str("invalid program Taproot tweak"),
            Self::OutputKeyMismatch => {
                formatter.write_str("known-tweak proof does not open the spent output key")
            }
        }
    }
}

impl std::error::Error for KnownTweakError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Derivation(error) => Some(error),
            _ => None,
        }
    }
}

// These are public scalars. SecretKey supplies the pinned curve arithmetic;
// zero needs explicit handling because it is not a valid secret key.
fn negate(value: Scalar) -> Scalar {
    if value == Scalar::ZERO {
        value
    } else {
        Scalar::from(
            SecretKey::from_slice(&value.to_be_bytes())
                .expect("nonzero canonical scalar")
                .negate(),
        )
    }
}

fn add(left: Scalar, right: Scalar) -> Scalar {
    if left == Scalar::ZERO {
        right
    } else if negate(left) == right {
        Scalar::ZERO
    } else {
        Scalar::from(
            SecretKey::from_slice(&left.to_be_bytes())
                .expect("nonzero canonical scalar")
                .add_tweak(&right)
                .expect("canonical scalars with a nonzero sum"),
        )
    }
}

fn with_sign(value: Scalar, parity: Parity) -> Scalar {
    if parity == Parity::Odd {
        negate(value)
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_scalar_arithmetic_handles_zero_and_cancellation() {
        assert_eq!(add(Scalar::ZERO, Scalar::ZERO), Scalar::ZERO);
        assert_eq!(add(Scalar::ONE, Scalar::ZERO), Scalar::ONE);
        assert_eq!(add(Scalar::ONE, Scalar::MAX), Scalar::ZERO);
        assert_eq!(negate(Scalar::ZERO), Scalar::ZERO);
        assert_eq!(negate(Scalar::ONE), Scalar::MAX);
    }
}
