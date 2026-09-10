// Copyright Judica, Inc 2026
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Public inputs for deterministic covenant lowering, without signer runtimes.

use crate::Clause;
use bitcoin::hashes::{sha256, Hash};
use bitcoin::util::bip32::{self, ChildNumber, ExtendedPubKey};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// A BIP-119 transaction-template predicate.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
#[schemars(transparent)]
pub struct Ctv(
    /// The expected default CTV hash.
    #[schemars(with = "String", regex(pattern = "^[0-9a-fA-F]{64}$"))]
    pub sha256::Hash,
);

/// Explicitly permit a predicate to use the caller's public lowering plan.
///
/// Only `Emulatable<Ctv>` currently has a policy implementation. This wrapper
/// does not imply an evaluator or signing protocol for arbitrary programs.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
#[schemars(transparent)]
pub struct Emulatable<P>(
    /// The predicate whose lowering may be selected explicitly.
    pub P,
);

/// Immutable, serializable choices used only by explicit emulatable predicates.
///
/// These inputs determine scripts without network access, runtime callbacks or
/// secret keys. Ordinary native clauses and raw scripts are not rewritten.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub enum LoweringPlan {
    /// Emit native CTV, assuming the deployment enforces its intended semantics.
    Native,
    /// Emit the existing CTV-specific BIP32 signer policy.
    CtvEmulation {
        /// Public roots in policy order; endpoint and transport settings are absent.
        #[schemars(with = "Vec<String>")]
        signers: Vec<ExtendedPubKey>,
        /// Required signer count, from one through the number of distinct roots.
        #[schemars(range(min = 1))]
        threshold: u8,
    },
}

/// A public lowering plan cannot produce the requested covenant policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CovenantError {
    /// The threshold is zero or exceeds the number of signers.
    InvalidThreshold {
        /// Requested threshold.
        threshold: u8,
        /// Number of configured roots.
        signers: usize,
    },
    /// Two roots have identical public key and chain code.
    DuplicateSigner {
        /// Earlier occurrence in the configured policy order.
        first: usize,
        /// Repeated occurrence, possibly with different descriptive metadata.
        second: usize,
    },
    /// The nine-child CTV path would overflow BIP32's depth byte.
    RootDepth {
        /// Root's index in the signer list.
        index: usize,
        /// Declared BIP32 depth.
        depth: u8,
    },
    /// BIP32 could not derive the exact requested child path.
    Derivation {
        /// Root's index in the signer list.
        index: usize,
        /// Underlying BIP32 failure; no alternate path is substituted.
        source: bip32::Error,
    },
}

impl fmt::Display for CovenantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidThreshold { threshold, signers } => write!(
                formatter,
                "CTV signer threshold {threshold} must be positive and no greater than {signers}"
            ),
            Self::DuplicateSigner { first, second } => write!(
                formatter,
                "CTV signers {first} and {second} have the same derivation identity"
            ),
            Self::RootDepth { index, depth } => write!(
                formatter,
                "CTV signer {index} has BIP32 depth {depth}; the maximum root depth is 246"
            ),
            Self::Derivation { index, source } => {
                write!(formatter, "CTV signer {index} derivation failed: {source}")
            }
        }
    }
}

impl std::error::Error for CovenantError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Derivation { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl LoweringPlan {
    /// Check threshold, independent derivation roots and BIP32 path depth.
    ///
    /// Public enum construction and deserialization can produce invalid plans;
    /// compilation must validate its inputs even if it has no CTV predicate.
    pub fn validate(&self) -> Result<(), CovenantError> {
        if let Self::CtvEmulation { signers, threshold } = self {
            if *threshold == 0 || usize::from(*threshold) > signers.len() {
                return Err(CovenantError::InvalidThreshold {
                    threshold: *threshold,
                    signers: signers.len(),
                });
            }
            let mut identities = BTreeMap::new();
            for (index, signer) in signers.iter().enumerate() {
                if signer.depth > u8::MAX - 9 {
                    return Err(CovenantError::RootDepth {
                        index,
                        depth: signer.depth,
                    });
                }
                // BIP32 derives from the compressed key and chain code. Network,
                // parent fingerprint and child-number labels add no independence.
                if let Some(first) =
                    identities.insert((signer.public_key, signer.chain_code), index)
                {
                    return Err(CovenantError::DuplicateSigner {
                        first,
                        second: index,
                    });
                }
            }
        }
        Ok(())
    }

    /// Lower an explicitly emulatable CTV predicate using public data alone.
    ///
    /// A single root produces a key clause; multiple roots retain the configured
    /// threshold and signer order, matching the existing HD/federation protocol.
    pub fn lower_ctv(&self, Ctv(hash): Ctv) -> Result<Clause, CovenantError> {
        self.validate()?;
        match self {
            Self::Native => Ok(Clause::TxTemplate(hash)),
            Self::CtvEmulation { signers, threshold } => {
                let path = hash_to_child_vec(hash);
                let mut clauses = signers
                    .iter()
                    .enumerate()
                    .map(|(index, signer)| {
                        crate::crypto::derive_public_key(signer, &path)
                            .map(Clause::Key)
                            .map_err(|source| CovenantError::Derivation { index, source })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if clauses.len() == 1 {
                    Ok(clauses.remove(0))
                } else {
                    Ok(Clause::Threshold(usize::from(*threshold), clauses))
                }
            }
        }
    }
}

/// Encode all 256 CTV hash bits as the existing nine non-hardened BIP32 children.
///
/// Each big-endian word contributes its low 31 bits to one of the first eight
/// children. Child nine stores their high bits, with word zero's bit at bit zero.
/// This preserves the existing CTV protocol exactly; it is not a namespaced
/// commitment scheme for arbitrary programs.
pub fn hash_to_child_vec(hash: sha256::Hash) -> Vec<ChildNumber> {
    let bytes = hash.into_inner();
    let mut children = Vec::with_capacity(9);
    let mut high_bits = 0;
    for (index, chunk) in bytes.chunks_exact(4).enumerate() {
        let word = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        children.push(ChildNumber::Normal {
            index: word & 0x7fff_ffff,
        });
        high_bits |= (word >> 31) << index;
    }
    children.push(ChildNumber::Normal { index: high_bits });
    children
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{PolicyCompiler, ScriptPolicy};
    use bitcoin::secp256k1::Secp256k1;
    use bitcoin::util::bip32::{ChainCode, ExtendedPrivKey, Fingerprint};
    use bitcoin::Network;
    use serde_json::json;
    use std::collections::BTreeSet;

    fn root(byte: u8) -> ExtendedPrivKey {
        ExtendedPrivKey::new_master(Network::Testnet, &[byte; 32]).unwrap()
    }

    fn public_root(byte: u8) -> ExtendedPubKey {
        ExtendedPubKey::from_priv(&Secp256k1::new(), &root(byte))
    }

    fn predicate() -> Ctv {
        Ctv(sha256::Hash::hash(b"public lowering"))
    }

    #[test]
    fn ctv_path_has_the_existing_fixed_word_and_high_bit_encoding() {
        let words: [u32; 8] = [
            0x8000_0000,
            0x7fff_ffff,
            0x1234_5678,
            0xabcd_ef01,
            0xffff_fff0,
            0,
            1,
            0x8000_00ff,
        ];
        let bytes: Vec<_> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
        let path = hash_to_child_vec(sha256::Hash::from_slice(&bytes).unwrap());
        assert_eq!(
            path.into_iter().map(u32::from).collect::<Vec<_>>(),
            [
                0,
                0x7fff_ffff,
                0x1234_5678,
                0x2bcd_ef01,
                0x7fff_fff0,
                0,
                1,
                0xff,
                0x99,
            ]
        );
    }

    #[test]
    fn every_hash_bit_survives_the_non_hardened_path_encoding() {
        let mut paths = BTreeSet::new();
        for bit in 0..256 {
            let mut bytes = [0; 32];
            bytes[bit / 8] = 1 << (bit % 8);
            let path = hash_to_child_vec(sha256::Hash::from_inner(bytes));
            assert_eq!(path.len(), 9);
            assert!(path.iter().all(ChildNumber::is_normal));
            let high = u32::from(path[8]);
            assert!(high <= 255);
            let restored: Vec<_> = path[..8]
                .iter()
                .enumerate()
                .flat_map(|(index, child)| {
                    (u32::from(*child) | (((high >> index) & 1) << 31)).to_be_bytes()
                })
                .collect();
            assert_eq!(restored, bytes);
            assert!(paths.insert(path));
        }
        assert_eq!(paths.len(), 256);
        assert_eq!(
            hash_to_child_vec(sha256::Hash::from_inner([0; 32])),
            vec![ChildNumber::Normal { index: 0 }; 9]
        );
        let maximum = hash_to_child_vec(sha256::Hash::from_inner([255; 32]));
        assert_eq!(
            maximum[..8],
            [ChildNumber::Normal { index: 0x7fff_ffff }; 8]
        );
        assert_eq!(maximum[8], ChildNumber::Normal { index: 255 });
    }

    #[test]
    fn public_lowering_agrees_with_private_signer_derivation_and_policy_order() {
        let secp = Secp256k1::new();
        let Ctv(hash) = predicate();
        let signers = vec![public_root(2), public_root(1)];
        let expected: Vec<_> = [2, 1]
            .into_iter()
            .map(|byte| {
                let private = root(byte)
                    .derive_priv(&secp, &hash_to_child_vec(hash))
                    .unwrap();
                Clause::Key(private.to_keypair(&secp).x_only_public_key().0)
            })
            .collect();
        assert_eq!(
            LoweringPlan::CtvEmulation {
                signers: signers[..1].to_vec(),
                threshold: 1,
            }
            .lower_ctv(Ctv(hash))
            .unwrap(),
            expected[0]
        );
        for threshold in [1, 2] {
            assert_eq!(
                LoweringPlan::CtvEmulation {
                    signers: signers.clone(),
                    threshold,
                }
                .lower_ctv(Ctv(hash))
                .unwrap(),
                Clause::Threshold(usize::from(threshold), expected.clone())
            );
        }
        assert_eq!(
            LoweringPlan::Native.lower_ctv(Ctv(hash)).unwrap(),
            Clause::TxTemplate(hash)
        );
    }

    #[test]
    fn invalid_thresholds_and_duplicate_derivation_roots_are_rejected() {
        for (signers, threshold) in [
            (vec![], 0),
            (vec![], 1),
            (vec![public_root(1)], 0),
            (vec![public_root(1)], 2),
        ] {
            let plan = LoweringPlan::CtvEmulation { signers, threshold };
            assert!(matches!(
                plan.validate(),
                Err(CovenantError::InvalidThreshold { .. })
            ));
            assert!(plan.lower_ctv(predicate()).is_err());
        }
        let root = public_root(1);
        let mut relabeled = root;
        relabeled.network = Network::Bitcoin;
        relabeled.depth = 12;
        relabeled.parent_fingerprint = Fingerprint::from(&[2; 4][..]);
        relabeled.child_number = ChildNumber::Normal { index: 42 };
        assert_ne!(root, relabeled);
        for duplicate in [root, relabeled] {
            assert_eq!(
                LoweringPlan::CtvEmulation {
                    signers: vec![root, duplicate],
                    threshold: 2,
                }
                .validate(),
                Err(CovenantError::DuplicateSigner {
                    first: 0,
                    second: 1
                })
            );
        }
        let mut independent = root;
        independent.chain_code = ChainCode::from(&[3; 32][..]);
        let plan = LoweringPlan::CtvEmulation {
            signers: vec![root, independent],
            threshold: 2,
        };
        plan.validate().unwrap();
        let Clause::Threshold(2, keys) = plan.lower_ctv(predicate()).unwrap() else {
            panic!("expected two-signer policy");
        };
        assert_ne!(keys[0], keys[1]);
    }

    #[test]
    fn bip32_root_depth_is_checked_before_derivation_can_overflow() {
        let mut root = public_root(1);
        root.depth = 246;
        LoweringPlan::CtvEmulation {
            signers: vec![root],
            threshold: 1,
        }
        .lower_ctv(predicate())
        .unwrap();
        for depth in [247, 255] {
            root.depth = depth;
            assert_eq!(
                LoweringPlan::CtvEmulation {
                    signers: vec![root],
                    threshold: 1,
                }
                .lower_ctv(predicate()),
                Err(CovenantError::RootDepth { index: 0, depth })
            );
        }
    }

    #[test]
    fn serialized_lowering_inputs_and_explicit_wrappers_round_trip() {
        assert_eq!(
            serde_json::to_value(LoweringPlan::Native).unwrap(),
            json!("Native")
        );
        let plan = LoweringPlan::CtvEmulation {
            signers: vec![public_root(1)],
            threshold: 1,
        };
        let value = serde_json::to_value(&plan).unwrap();
        assert_eq!(
            value,
            json!({"CtvEmulation": {"signers": [public_root(1).to_string()], "threshold": 1}})
        );
        let restored: LoweringPlan = serde_json::from_value(value).unwrap();
        assert_eq!(restored, plan);
        assert_eq!(
            restored.lower_ctv(predicate()).unwrap(),
            plan.lower_ctv(predicate()).unwrap()
        );

        // BIP32 has one tpub version for all test networks. Deserialization
        // labels it Testnet; that metadata does not change the derived policy.
        let mut regtest_root = public_root(1);
        regtest_root.network = Network::Regtest;
        let regtest_plan = LoweringPlan::CtvEmulation {
            signers: vec![regtest_root],
            threshold: 1,
        };
        assert_eq!(
            serde_json::to_value(&regtest_plan).unwrap(),
            serde_json::to_value(&plan).unwrap()
        );
        assert_eq!(
            regtest_plan.lower_ctv(predicate()).unwrap(),
            restored.lower_ctv(predicate()).unwrap()
        );

        let source = Emulatable(predicate()).compile_policy().unwrap();
        assert_eq!(source, ScriptPolicy::Emulatable(Emulatable(predicate())));
        let serialized = serde_json::to_value(&source).unwrap();
        assert_eq!(serialized, json!({"Emulatable": predicate().0.to_string()}));
        assert_eq!(
            serde_json::from_value::<ScriptPolicy>(serialized).unwrap(),
            source
        );
        assert_ne!(
            source,
            Clause::TxTemplate(predicate().0).compile_policy().unwrap()
        );
        // Both are usable by the guest's schema generation without a host runtime.
        serde_json::to_value(schemars::schema_for!(LoweringPlan)).unwrap();
        serde_json::to_value(schemars::schema_for!(ScriptPolicy)).unwrap();
    }

    #[test]
    fn malformed_serialized_keys_and_predicates_are_rejected() {
        assert!(
            serde_json::from_value::<LoweringPlan>(json!({"CtvEmulation": {
                "signers": ["not an extended public key"], "threshold": 1
            }}))
            .is_err()
        );
        assert!(serde_json::from_value::<ScriptPolicy>(json!({"Emulatable": "00"})).is_err());
        assert!(
            serde_json::from_value::<LoweringPlan>(json!({"CtvEmulation": {
                "signers": [public_root(1).to_string()], "threshold": 1, "endpoint": "hidden"
            }}))
            .is_err()
        );
    }
}
