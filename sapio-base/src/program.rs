// Copyright Judica, Inc 2026
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Exact program instances and public keys for explicitly evaluated emulation.
//!
//! Instance commitments identify evaluator semantics and the complete program
//! and parameter bytes. They do not provide an interpreter or attest that an
//! oracle implements those semantics honestly. Public derivation is independent
//! of signer transports, evaluator registration and the CTV lowering plan.

use crate::covenant::{hash_to_child_vec, Ctv};
use crate::policy::{PolicyCompiler, PolicyError, ScriptPolicy};
use crate::Clause;
use bitcoin::hashes::{sha256, Hash, HashEngine};
use bitcoin::util::bip32::{self, ChildNumber, ExtendedPubKey};
use bitcoin::XOnlyPublicKey;
use schemars::JsonSchema;
use serde::de::{Error as _, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

/// Maximum exact program length, in bytes.
pub const MAX_PROGRAM_BYTES: usize = 65_536;
/// Maximum exact preset-parameter length, in bytes.
pub const MAX_PARAMETER_BYTES: usize = 65_536;
/// Maximum root depth before deriving the ten-child program path.
pub const MAX_PROGRAM_ROOT_DEPTH: u8 = u8::MAX - 10;

const PROGRAM_NAMESPACE: u32 = 0x5341_5049;
const COMMITMENT_TAG: &[u8] = b"Sapio/Emulation/Program/v1";
const EVALUATOR_TAG: &[u8] = b"Sapio/Emulation/Evaluator/Wasm/v1";

/// The exact compiled CTV evaluator distributed with this Sapio version.
///
/// Rebuilding different bytes changes the program identity and derived key.
/// Sources and a byte-for-byte verification script live in `evaluators/`.
pub const CTV_WASM: &[u8] = include_bytes!("../../evaluators/artifacts/ctv.wasm");
const _: () = assert!(CTV_WASM.len() <= MAX_PROGRAM_BYTES);

/// Construct the inline WASM CTV predicate for an expected BIP119 hash.
///
/// This evaluator requires every input to spend a native witness program,
/// establishing empty scriptSigs by consensus. Legacy and P2SH inputs are
/// unsupported. Parameters are the expected 32 hash bytes; evidence is empty.
/// Its key commits to the complete module and differs from the older
/// nine-child CTV emulation key.
pub fn ctv_wasm_instance(Ctv(hash): Ctv) -> ProgramInstance {
    ProgramInstance {
        evaluator: EvaluatorId::wasm(),
        program: CTV_WASM.to_vec(),
        parameters: hash.as_ref().to_vec(),
    }
}

/// An identifier for exact evaluator and transaction-view semantics.
///
/// The evaluator protocol must define what these bytes identify. A friendly
/// name, executable path or registration order does not establish semantics.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
#[schemars(transparent)]
pub struct EvaluatorId(
    /// The evaluator protocol's identifier.
    #[schemars(with = "String", regex(pattern = "^[0-9a-fA-F]{64}$"))]
    pub sha256::Hash,
);

impl EvaluatorId {
    /// Execute the instance's program bytes directly as a WASM-v1 evaluator.
    ///
    /// The all-zero identity permanently selects the version-one evaluator
    /// ABI, signed view and metered host operations. It cannot be registered
    /// or overridden by an oracle operator.
    pub fn wasm() -> Self {
        Self(sha256::Hash::from_inner([0; 32]))
    }

    /// Whether this is the reserved inline WASM-v1 identity.
    pub fn is_wasm(self) -> bool {
        self == Self::wasm()
    }

    /// Identify a registered WASM-v1 interpreter by its exact module bytes.
    ///
    /// The tagged hash is SHA256(tag || tag || module), where tag is SHA256
    /// of `Sapio/Emulation/Evaluator/Wasm/v1`. The instance's program bytes
    /// become input to this interpreter. This helper does not validate WASM
    /// or register executable code with an oracle.
    pub fn for_wasm(module: &[u8]) -> Self {
        let tag = sha256::Hash::hash(EVALUATOR_TAG);
        let mut engine = sha256::Hash::engine();
        engine.input(&tag[..]);
        engine.input(&tag[..]);
        engine.input(module);
        Self(sha256::Hash::from_engine(engine))
    }
}

impl Default for EvaluatorId {
    fn default() -> Self {
        Self::wasm()
    }
}

/// A domain-separated commitment to one complete program instance.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
#[schemars(transparent)]
pub struct ProgramId(
    /// The committed instance hash.
    #[schemars(with = "String", regex(pattern = "^[0-9a-fA-F]{64}$"))]
    pub sha256::Hash,
);

/// An immutable, bounded program and its preset parameters.
///
/// Deserialization enforces the same limits as [`Self::new`]. Evaluators must
/// separately validate program and parameter encodings and bound their work.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
pub struct ProgramInstance {
    evaluator: EvaluatorId,
    #[schemars(length(max = 65536))]
    program: Vec<u8>,
    #[schemars(length(max = 65536))]
    parameters: Vec<u8>,
}

impl ProgramInstance {
    /// Commit an inline WASM-v1 evaluator and its fixed parameters.
    ///
    /// The module itself is the program. At execution its separate program
    /// input is empty; preset parameters and signed transaction data remain
    /// distinct ABI arguments.
    pub fn wasm(module: Vec<u8>, parameters: Vec<u8>) -> Result<Self, ProgramError> {
        Self::new(EvaluatorId::wasm(), module, parameters)
    }

    /// Construct an exact instance after checking both byte-length limits.
    pub fn new(
        evaluator: EvaluatorId,
        program: Vec<u8>,
        parameters: Vec<u8>,
    ) -> Result<Self, ProgramError> {
        if program.len() > MAX_PROGRAM_BYTES {
            return Err(ProgramError::ProgramTooLarge {
                size: program.len(),
            });
        }
        if parameters.len() > MAX_PARAMETER_BYTES {
            return Err(ProgramError::ParametersTooLarge {
                size: parameters.len(),
            });
        }
        Ok(Self {
            evaluator,
            program,
            parameters,
        })
    }

    /// Return the exact evaluator identifier.
    pub fn evaluator(&self) -> EvaluatorId {
        self.evaluator
    }

    /// Borrow the exact program bytes.
    pub fn program(&self) -> &[u8] {
        &self.program
    }

    /// Borrow the exact preset parameters.
    pub fn parameters(&self) -> &[u8] {
        &self.parameters
    }

    /// Commit every identity component using the version-one tagged encoding.
    ///
    /// The tag is `Sapio/Emulation/Program/v1`. Its SHA256 digest is prepended
    /// twice, followed by the 32 evaluator bytes, a four-byte little-endian
    /// program length, program bytes, a four-byte little-endian parameter
    /// length, and parameter bytes. JSON formatting is never hashed.
    pub fn id(&self) -> ProgramId {
        let tag = sha256::Hash::hash(COMMITMENT_TAG);
        let mut engine = sha256::Hash::engine();
        engine.input(&tag[..]);
        engine.input(&tag[..]);
        engine.input(&self.evaluator.0[..]);
        engine.input(&(self.program.len() as u32).to_le_bytes());
        engine.input(&self.program);
        engine.input(&(self.parameters.len() as u32).to_le_bytes());
        engine.input(&self.parameters);
        ProgramId(sha256::Hash::from_engine(engine))
    }

    /// Derive this instance's signature key using only the public root.
    ///
    /// Under the same configured root, the fixed ten-child namespace is
    /// separate from CTV's nine-child path. Failed derivation never selects
    /// an alternate path.
    pub fn derive_public_key(&self, root: &ExtendedPubKey) -> Result<XOnlyPublicKey, ProgramError> {
        if root.depth > MAX_PROGRAM_ROOT_DEPTH {
            return Err(ProgramError::RootDepth { depth: root.depth });
        }
        crate::crypto::derive_public_key(root, &program_derivation_path(self.id()))
            .map_err(ProgramError::Derivation)
    }
}

impl<'de> Deserialize<'de> for ProgramInstance {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            evaluator: EvaluatorId,
            #[serde(deserialize_with = "bounded_program")]
            program: Vec<u8>,
            #[serde(deserialize_with = "bounded_parameters")]
            parameters: Vec<u8>,
        }
        let Fields {
            evaluator,
            program,
            parameters,
        } = Fields::deserialize(deserializer)?;
        Self::new(evaluator, program, parameters).map_err(D::Error::custom)
    }
}

fn bounded_program<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    bounded_bytes::<D, MAX_PROGRAM_BYTES>(deserializer)
}

fn bounded_parameters<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    bounded_bytes::<D, MAX_PARAMETER_BYTES>(deserializer)
}

fn bounded_bytes<'de, D: Deserializer<'de>, const MAX: usize>(
    deserializer: D,
) -> Result<Vec<u8>, D::Error> {
    struct Bytes<const MAX: usize>;
    impl<'de, const MAX: usize> Visitor<'de> for Bytes<MAX> {
        type Value = Vec<u8>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "an array of at most {MAX} bytes")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut bytes = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(MAX));
            while let Some(byte) = seq.next_element()? {
                if bytes.len() == MAX {
                    return Err(A::Error::custom(format!("byte array exceeds {MAX} bytes")));
                }
                bytes.push(byte);
            }
            Ok(bytes)
        }
    }
    deserializer.deserialize_seq(Bytes::<MAX>)
}

/// Encode an instance ID in the version-one generic-program BIP32 namespace.
///
/// Child one is the normal index `0x53415049` (`SAPI`), followed by the existing
/// lossless nine-child hash encoding. CTV uses nine children without this
/// prefix under the same root. Every child is non-hardened so public and
/// private derivation agree.
pub fn program_derivation_path(id: ProgramId) -> Vec<ChildNumber> {
    let mut path = Vec::with_capacity(10);
    path.push(ChildNumber::Normal {
        index: PROGRAM_NAMESPACE,
    });
    path.extend(hash_to_child_vec(id.0));
    path
}

/// A complete program instance and the public root authorized to evaluate it.
///
/// This custom [`PolicyCompiler`] produces an ordinary signature-key clause.
/// It does not register an evaluator, contact a signer, or add its assumptions
/// to the compiled object's CTV-specific covenant requirements. Applications
/// must retain this complete value for their evaluated signing requests.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct EmulatedProgram {
    instance: ProgramInstance,
    #[schemars(with = "String")]
    root: ExtendedPubKey,
}

impl EmulatedProgram {
    /// Check that this public root can derive the exact instance key.
    pub fn new(instance: ProgramInstance, root: ExtendedPubKey) -> Result<Self, ProgramError> {
        instance.derive_public_key(&root)?;
        Ok(Self { instance, root })
    }

    /// Borrow the complete program instance.
    pub fn instance(&self) -> &ProgramInstance {
        &self.instance
    }

    /// Borrow the explicitly selected public oracle root.
    pub fn root(&self) -> &ExtendedPubKey {
        &self.root
    }

    /// Derive the exact key used by this policy without a signer runtime.
    pub fn derive_public_key(&self) -> Result<XOnlyPublicKey, ProgramError> {
        self.instance.derive_public_key(&self.root)
    }
}

impl<'de> Deserialize<'de> for EmulatedProgram {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            instance: ProgramInstance,
            root: ExtendedPubKey,
        }
        let Fields { instance, root } = Fields::deserialize(deserializer)?;
        Self::new(instance, root).map_err(D::Error::custom)
    }
}

impl PolicyCompiler for EmulatedProgram {
    fn compile_policy(&self) -> Result<ScriptPolicy, PolicyError> {
        self.derive_public_key()
            .map(|key| ScriptPolicy::Miniscript(Clause::Key(key)))
            .map_err(|error| PolicyError::Backend(error.to_string()))
    }
}

/// An exact program instance or its public derivation is invalid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProgramError {
    /// The exact program exceeds the public size limit.
    ProgramTooLarge {
        /// Actual program length.
        size: usize,
    },
    /// The preset parameters exceed the public size limit.
    ParametersTooLarge {
        /// Actual parameter length.
        size: usize,
    },
    /// Ten child derivations would overflow BIP32's depth byte.
    RootDepth {
        /// The supplied root's BIP32 depth.
        depth: u8,
    },
    /// BIP32 cannot derive the exact requested child path.
    Derivation(bip32::Error),
}

impl fmt::Display for ProgramError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProgramTooLarge { size } => write!(
                formatter,
                "program has {size} bytes; maximum is {MAX_PROGRAM_BYTES}"
            ),
            Self::ParametersTooLarge { size } => write!(
                formatter,
                "parameters have {size} bytes; maximum is {MAX_PARAMETER_BYTES}"
            ),
            Self::RootDepth { depth } => write!(
                formatter,
                "program signer root has depth {depth}; maximum is {MAX_PROGRAM_ROOT_DEPTH}"
            ),
            Self::Derivation(error) => write!(formatter, "program key derivation failed: {error}"),
        }
    }
}

impl std::error::Error for ProgramError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Derivation(error) => Some(error),
            _ => None,
        }
    }
}
