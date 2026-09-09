// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Checked Taproot spending data for leaves outside the Miniscript language.

use bitcoin::secp256k1::Secp256k1;
use bitcoin::util::taproot::{TaprootBuilder, TaprootBuilderError, TaprootSpendInfo};
use bitcoin::{Script, XOnlyPublicKey};
use sapio_base::policy::{validate_tapscript, PolicyError};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};
use std::fmt;

/// Maximum number of explicit script leaves in a raw Taproot artifact.
pub const MAX_TAPROOT_LEAVES: usize = 1024;

/// A checked tree of TapScript leaves in depth-first order.
///
/// Serialized data contains only the internal key and ordered `(depth, script)`
/// leaves. The output key and control blocks are reconstructed on decoding.
/// Script checks establish structural validity, not satisfiability or a witness
/// weight bound. An empty leaf list explicitly represents a key-only output.
#[derive(Serialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RawTaproot {
    #[schemars(with = "String", regex(pattern = "^[0-9a-fA-F]{64}$"))]
    internal_key: XOnlyPublicKey,
    #[schemars(length(max = "MAX_TAPROOT_LEAVES"))]
    leaves: Vec<(u8, Script)>,
    #[serde(skip)]
    #[schemars(skip)]
    spend_info: TaprootSpendInfo,
}

/// Why raw Taproot spending data cannot be accepted.
#[derive(Debug)]
pub enum RawTaprootError {
    /// The artifact exceeds the leaf-count bound.
    TooManyLeaves(usize),
    /// An individual leaf violates the shared TapScript structure rules.
    InvalidLeaf {
        /// Index in the serialized depth-first leaf list.
        index: usize,
        /// The failed script invariant.
        error: PolicyError,
    },
    /// The leaf depths do not describe a complete, valid Taproot tree.
    InvalidTree(TaprootBuilderError),
}

impl fmt::Display for RawTaprootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyLeaves(count) => write!(
                f,
                "raw Taproot tree has {count} leaves; maximum is {MAX_TAPROOT_LEAVES}"
            ),
            Self::InvalidLeaf { index, error } => {
                write!(f, "invalid raw Taproot leaf {index}: {error}")
            }
            Self::InvalidTree(error) => write!(f, "invalid raw Taproot tree: {error}"),
        }
    }
}

impl std::error::Error for RawTaprootError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::TooManyLeaves(_) => None,
            Self::InvalidLeaf { error, .. } => Some(error),
            Self::InvalidTree(error) => Some(error),
        }
    }
}

impl RawTaproot {
    /// Build a deterministic Huffman tree with equal weight for each distinct
    /// script. Only identical complete leaves are deduplicated; script bytes
    /// retain their exact instruction order and multiplicity.
    pub fn from_scripts(
        internal_key: XOnlyPublicKey,
        scripts: Vec<Script>,
    ) -> Result<Self, RawTaprootError> {
        #[derive(PartialEq, Eq, PartialOrd, Ord)]
        enum Tree {
            Leaf(Script),
            Branch(Box<Tree>, Box<Tree>),
        }

        let scripts: BTreeSet<_> = scripts.into_iter().collect();
        if scripts.len() > MAX_TAPROOT_LEAVES {
            return Err(RawTaprootError::TooManyLeaves(scripts.len()));
        }
        // Weight and lexicographic tree order make ties independent of caller
        // order. Equal leaf weights keep the maximum depth at ten for 1024
        // leaves, well below Taproot's limit of 128.
        let mut heap: BinaryHeap<_> = scripts
            .into_iter()
            .map(|script| Reverse((1usize, Tree::Leaf(script))))
            .collect();
        while heap.len() > 1 {
            let Reverse((left_weight, left)) = heap.pop().expect("at least two trees");
            let Reverse((right_weight, right)) = heap.pop().expect("at least two trees");
            heap.push(Reverse((
                left_weight + right_weight,
                Tree::Branch(Box::new(left), Box::new(right)),
            )));
        }
        let mut leaves = vec![];
        if let Some(Reverse((_, tree))) = heap.pop() {
            let mut pending = vec![(0, tree)];
            while let Some((depth, tree)) = pending.pop() {
                match tree {
                    Tree::Leaf(script) => leaves.push((depth, script)),
                    Tree::Branch(left, right) => {
                        pending.push((depth + 1, *right));
                        pending.push((depth + 1, *left));
                    }
                }
            }
        }
        Self::new(internal_key, leaves)
    }

    /// Validate scripts and tree shape, and derive the output and spending data.
    /// Every leaf uses the current TapScript leaf version; hidden subtrees and
    /// future leaf versions cannot be introduced through this representation.
    pub fn new(
        internal_key: XOnlyPublicKey,
        leaves: Vec<(u8, Script)>,
    ) -> Result<Self, RawTaprootError> {
        if leaves.len() > MAX_TAPROOT_LEAVES {
            return Err(RawTaprootError::TooManyLeaves(leaves.len()));
        }
        let mut builder = TaprootBuilder::new();
        for (index, (depth, script)) in leaves.iter().enumerate() {
            validate_tapscript(script)
                .map_err(|error| RawTaprootError::InvalidLeaf { index, error })?;
            builder = builder
                .add_leaf(*depth, script.clone())
                .map_err(RawTaprootError::InvalidTree)?;
        }
        // This bitcoin fork's finalize() does not itself require all pending
        // branches to have been combined. Check completion before calling it.
        if !leaves.is_empty() && !builder.is_finalized() {
            return Err(RawTaprootError::InvalidTree(
                TaprootBuilderError::IncompleteTree,
            ));
        }
        let spend_info = builder
            .finalize(&Secp256k1::verification_only(), internal_key)
            .map_err(RawTaprootError::InvalidTree)?;
        Ok(Self {
            internal_key,
            leaves,
            spend_info,
        })
    }

    /// The untweaked internal key committed to by this output.
    pub fn internal_key(&self) -> XOnlyPublicKey {
        self.internal_key
    }

    /// The exact depth-first leaf list, with depths measured from root zero.
    pub fn leaves(&self) -> &[(u8, Script)] {
        &self.leaves
    }

    /// Derived output key, Merkle root and script-path control blocks.
    pub fn spend_info(&self) -> &TaprootSpendInfo {
        &self.spend_info
    }

    /// The scriptPubKey committing to the checked internal key and script tree.
    pub fn script_pubkey(&self) -> Script {
        Script::new_v1_p2tr_tweaked(self.spend_info.output_key())
    }
}

impl<'de> Deserialize<'de> for RawTaproot {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Data {
            internal_key: XOnlyPublicKey,
            leaves: Vec<(u8, Script)>,
        }

        let data = Data::deserialize(deserializer)?;
        Self::new(data.internal_key, data.leaves).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::{Keypair, SecretKey};
    use bitcoin::util::taproot::LeafVersion;
    use serde_json::json;

    fn key() -> XOnlyPublicKey {
        Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[1; 32]).unwrap())
            .x_only_public_key()
            .0
    }

    fn script(bytes: &[u8]) -> Script {
        Script::from(bytes.to_vec())
    }

    #[test]
    fn round_trip_reconstructs_the_same_output_and_control_blocks() {
        // The second leaf uses a balanced altstack, as Miniscript can do.
        let leaves = vec![(1, script(&[0x51])), (1, script(&[0x51, 0x6b, 0x6c]))];
        let raw = RawTaproot::new(key(), leaves.clone()).unwrap();
        let serialized = serde_json::to_value(&raw).unwrap();
        assert_eq!(serialized.as_object().unwrap().len(), 2);
        let decoded: RawTaproot = serde_json::from_value(serialized).unwrap();
        assert_eq!(decoded, raw);
        assert_eq!(decoded.internal_key(), key());
        assert_eq!(decoded.leaves(), leaves);
        assert_eq!(decoded.script_pubkey(), raw.script_pubkey());
        for (_, script) in leaves {
            let control = decoded
                .spend_info()
                .control_block(&(script.clone(), LeafVersion::TapScript))
                .unwrap();
            assert!(control.verify_taproot_commitment(
                &Secp256k1::verification_only(),
                decoded.spend_info().output_key().to_inner(),
                &script,
            ));
        }
        assert_eq!(raw.spend_info().as_script_map().len(), 2);
    }

    #[test]
    fn an_explicit_key_only_tree_has_no_script_path() {
        let raw = RawTaproot::new(key(), vec![]).unwrap();
        assert_eq!(raw.spend_info().merkle_root(), None);
        assert!(raw.spend_info().as_script_map().is_empty());
        assert_eq!(
            raw.script_pubkey(),
            Script::new_v1_p2tr(&Secp256k1::verification_only(), key(), None)
        );
        let decoded: RawTaproot =
            serde_json::from_str(&serde_json::to_string(&raw).unwrap()).unwrap();
        assert_eq!(decoded, raw);
    }

    #[test]
    fn script_factory_is_canonical_and_preserves_distinct_script_bytes() {
        let first = script(&[0x51]);
        let second = script(&[0x52]);
        let third = script(&[0x51, 0x75, 0x51]);
        let raw = RawTaproot::from_scripts(
            key(),
            vec![first.clone(), second.clone(), first.clone(), third.clone()],
        )
        .unwrap();
        let permuted =
            RawTaproot::from_scripts(key(), vec![third.clone(), second.clone(), first.clone()])
                .unwrap();
        assert_eq!(raw, permuted);
        assert_eq!(raw.leaves().len(), 3);
        assert_eq!(
            raw.leaves()
                .iter()
                .map(|(_, script)| script.clone())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([first, second, third])
        );
        let round_trip: RawTaproot =
            serde_json::from_slice(&serde_json::to_vec(&raw).unwrap()).unwrap();
        assert_eq!(round_trip.spend_info(), raw.spend_info());
        assert_eq!(
            RawTaproot::from_scripts(key(), vec![]).unwrap(),
            RawTaproot::new(key(), vec![]).unwrap()
        );
    }

    #[test]
    fn incomplete_out_of_order_and_overcomplete_trees_are_rejected() {
        for depths in [vec![1], vec![1, 2], vec![2, 1, 2], vec![0, 0]] {
            let leaves = depths.into_iter().map(|depth| (depth, script(&[0x51])));
            assert!(matches!(
                RawTaproot::new(key(), leaves.collect()),
                Err(RawTaprootError::InvalidTree(_))
            ));
        }
    }

    #[test]
    fn tree_depth_and_leaf_count_boundaries_are_checked() {
        let mut deepest: Vec<_> = (1..=128).map(|depth| (depth, script(&[0x51]))).collect();
        deepest.push((128, script(&[0x51])));
        assert!(RawTaproot::new(key(), deepest).is_ok());
        assert!(matches!(
            RawTaproot::new(key(), vec![(129, script(&[0x51]))]),
            Err(RawTaprootError::InvalidTree(
                TaprootBuilderError::InvalidMerkleTreeDepth(129)
            ))
        ));
        let leaves = vec![(10, script(&[0x51])); MAX_TAPROOT_LEAVES];
        assert!(RawTaproot::new(key(), leaves.clone()).is_ok());
        let mut too_many = leaves;
        too_many.push((10, script(&[0x51])));
        assert!(matches!(
            RawTaproot::new(key(), too_many),
            Err(RawTaprootError::TooManyLeaves(1025))
        ));
    }

    #[test]
    fn decoding_rechecks_tree_shape_scripts_and_derived_fields() {
        let raw = RawTaproot::new(key(), vec![(0, script(&[0x51]))]).unwrap();
        let serialized = serde_json::to_value(&raw).unwrap();
        let mut incomplete = serialized.clone();
        incomplete["leaves"] = json!([(1, script(&[0x51]))]);
        assert!(serde_json::from_value::<RawTaproot>(incomplete).is_err());

        for bytes in [vec![0x4c], vec![0x50], vec![0xab], vec![0x63, 0x51]] {
            let mut invalid_script = serialized.clone();
            invalid_script["leaves"] = json!([(0, script(&bytes))]);
            assert!(serde_json::from_value::<RawTaproot>(invalid_script).is_err());
        }
        let mut injected_cache = serialized;
        injected_cache["spend_info"] = json!({});
        assert!(serde_json::from_value::<RawTaproot>(injected_cache).is_err());
    }
}
