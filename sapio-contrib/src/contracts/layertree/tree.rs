//! Dense MID LayerTree records and authenticated Merkle-sum paths.
//!
//! Encoding follows LayerTree v0.15.2, sections 5 and 6 (BSD-3-Clause):
//! https://github.com/roasbeef/layertree/tree/f64cc0e4d4471e32f7fd8294ecb0b1c7547c1166

use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{Secp256k1, XOnlyPublicKey};
use bitcoin::{ScriptBuf, XOnlyPublicKey as Key};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// MID arithmetic uses nonnegative four-byte CScriptNum operands.
pub const MAX_SUM: u64 = (1 << 31) - 1;
/// The MID non-claimable marker differs from FULL's unsigned marker.
pub const SEQ_FINAL: u32 = 0x7fff_ffff;
/// BIP341's provably unspendable H point, as an even x-only key.
pub const NUMS: [u8; 32] = [
    0x50, 0x92, 0x9b, 0x74, 0xc1, 0xa0, 0x49, 0x54, 0xb7, 0x8b, 0x4b, 0x60, 0x35, 0xe9, 0x7a, 0x5e,
    0x07, 0x8a, 0x5a, 0x0f, 0x28, 0xec, 0x96, 0xd5, 0x47, 0xbf, 0xee, 0x9a, 0xce, 0x80, 0x3a, 0xc0,
];

/// Compute BIP340's tagged SHA256 construction.
pub fn tagged_hash(tag: &str, message: &[u8]) -> [u8; 32] {
    let prefix = sha256::Hash::hash(tag.as_bytes()).to_byte_array();
    let mut input = Vec::with_capacity(64 + message.len());
    input.extend_from_slice(&prefix);
    input.extend_from_slice(&prefix);
    input.extend_from_slice(message);
    sha256::Hash::hash(&input).to_byte_array()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// A fixed 116-byte member record in a dense LayerTree.
pub struct Leaf {
    /// Balance in satoshis, strictly positive and within the MID range.
    pub amount: u64,
    /// Output key paid by exits and evictions.
    pub exit_key: Key,
    /// Tree authority for owner exits, updates and vouchers.
    pub owner_key: Key,
    /// Authority permitted to force an exact payout after the lease.
    pub evictor_key: Key,
    /// Earliest eviction block height.
    pub evict_after: u32,
    /// Together with the owner, identifies a leaf within its lineage.
    pub nonce: u32,
    /// Voucher generation; incremented by every carve and never wrapped.
    pub seq: u32,
}

impl Leaf {
    /// Check the supported profile's record invariants.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.amount == 0 || self.amount > MAX_SUM {
            return Err("leaf amount is outside the MID range");
        }
        if self.evict_after >= 500_000_000 {
            return Err("leaf lease must be a block height");
        }
        if self.seq > SEQ_FINAL {
            return Err("leaf sequence is outside the MID range");
        }
        Ok(())
    }

    /// Encode the canonical fixed-width leaf, independent of witness numerics.
    pub fn serialize(&self) -> [u8; 116] {
        let mut bytes = [0; 116];
        bytes[..8].copy_from_slice(&self.amount.to_le_bytes());
        bytes[8..40].copy_from_slice(&self.exit_key.serialize());
        bytes[40..72].copy_from_slice(&self.owner_key.serialize());
        bytes[72..104].copy_from_slice(&self.evictor_key.serialize());
        bytes[104..108].copy_from_slice(&self.evict_after.to_le_bytes());
        bytes[108..112].copy_from_slice(&self.nonce.to_le_bytes());
        bytes[112..].copy_from_slice(&self.seq.to_le_bytes());
        bytes
    }

    /// Commit the complete leaf and expose its balance as its node sum.
    pub fn node(&self) -> Node {
        Node {
            hash: tagged_hash("LayerTree/leaf", &self.serialize()),
            sum: self.amount,
        }
    }

    /// Vouchers bind the body, allowing an amount-only top-up to survive.
    pub fn voucher_message(&self, tree_id: [u8; 32], claim: &Leaf) -> [u8; 32] {
        self.voucher_message_inner(tree_id, claim, None)
    }

    /// Bind a voucher to a payment hash under its distinct domain tag.
    pub fn hashlocked_voucher_message(
        &self,
        tree_id: [u8; 32],
        claim: &Leaf,
        payment_hash: [u8; 32],
    ) -> [u8; 32] {
        self.voucher_message_inner(tree_id, claim, Some(payment_hash))
    }

    fn voucher_message_inner(
        &self,
        tree_id: [u8; 32],
        claim: &Leaf,
        payment_hash: Option<[u8; 32]>,
    ) -> [u8; 32] {
        let mut body = Vec::with_capacity(288);
        body.extend_from_slice(&tree_id);
        body.extend_from_slice(&self.serialize()[8..]);
        body.extend_from_slice(&claim.serialize());
        let tag = if let Some(hash) = payment_hash {
            body.extend_from_slice(&hash);
            "LayerTree/voucher-hashlock"
        } else {
            "LayerTree/voucher"
        };
        tagged_hash(tag, &body)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
/// An authenticated hash and its recomputed balance sum.
pub struct Node {
    /// Tagged leaf or branch hash; the empty leaf is zero.
    pub hash: [u8; 32],
    /// Balance sum in satoshis.
    pub sum: u64,
}

impl Node {
    /// Encode hash32 followed by the nine-byte enc64 sum.
    pub fn serialize(&self) -> [u8; 41] {
        let mut bytes = [0; 41];
        bytes[..32].copy_from_slice(&self.hash);
        bytes[32..40].copy_from_slice(&self.sum.to_le_bytes());
        bytes[40] = 1;
        bytes
    }

    /// Hash ordered children and reject sum overflow before committing it.
    pub fn parent(left: Self, right: Self) -> Result<Self, &'static str> {
        let sum = left
            .sum
            .checked_add(right.sum)
            .filter(|n| *n <= MAX_SUM)
            .ok_or("Merkle-sum total exceeds MID range")?;
        let mut bytes = [0; 82];
        bytes[..41].copy_from_slice(&left.serialize());
        bytes[41..].copy_from_slice(&right.serialize());
        Ok(Self {
            hash: tagged_hash("LayerTree/node", &bytes),
            sum,
        })
    }

    /// Derive the canonical empty subtree at this height.
    pub fn empty(height: u8) -> Self {
        let mut node = Self::default();
        for _ in 0..height {
            node = Self::parent(node, node).expect("empty nodes have zero sum");
        }
        node
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// One bottom-up sibling in an inclusion or replacement proof.
pub struct Sibling {
    /// True when the node being proven is the right child.
    pub right: bool,
    /// Authenticated sibling hash and sum.
    pub node: Node,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// A bottom-up sequence of siblings and child directions.
pub struct Proof(pub Vec<Sibling>);

impl Proof {
    /// Recompute the root and total for the supplied starting node.
    pub fn fold(&self, mut node: Node) -> Result<Node, &'static str> {
        for sibling in &self.0 {
            node = if sibling.right {
                Node::parent(sibling.node, node)?
            } else {
                Node::parent(node, sibling.node)?
            };
        }
        Ok(node)
    }
}

/// Sparse storage for a dense, operator-positioned complete binary tree.
#[derive(Clone, Debug)]
pub struct Tree {
    depth: u8,
    leaves: BTreeMap<u32, Leaf>,
    nodes: BTreeMap<(u8, u32), Node>,
}

impl Tree {
    /// Create an empty tree in the supported depth range.
    pub fn new(depth: u8) -> Result<Self, &'static str> {
        if !(1..=16).contains(&depth) {
            return Err("supported depth is 1 through 16");
        }
        Ok(Self {
            depth,
            leaves: BTreeMap::new(),
            nodes: BTreeMap::new(),
        })
    }

    /// Fixed number of levels from leaf to root.
    pub fn depth(&self) -> u8 {
        self.depth
    }
    /// Read the member record at a position, if occupied.
    pub fn leaf(&self, position: u32) -> Option<&Leaf> {
        self.leaves.get(&position)
    }
    /// Iterate occupied positions in ascending order.
    pub fn leaves(&self) -> impl Iterator<Item = (u32, &Leaf)> {
        self.leaves.iter().map(|(p, l)| (*p, l))
    }
    /// Return the current root commitment and total.
    pub fn root(&self) -> Node {
        self.subtree(self.depth, 0).expect("root position is valid")
    }

    /// Read the aligned subtree identified by height and index.
    pub fn subtree(&self, height: u8, index: u32) -> Result<Node, &'static str> {
        if height > self.depth || index >= (1u32 << (self.depth - height)) {
            return Err("subtree position is outside the tree");
        }
        Ok(self
            .nodes
            .get(&(height, index))
            .copied()
            .unwrap_or_else(|| Node::empty(height)))
    }

    /// Validate the complete update before changing the tree.
    pub fn set(&mut self, position: u32, leaf: Option<Leaf>) -> Result<(), &'static str> {
        if position >= 1u32 << self.depth {
            return Err("leaf position is outside the tree");
        }
        if let Some(leaf) = &leaf {
            leaf.validate()?;
            if self.leaves.iter().any(|(p, old)| {
                *p != position && old.owner_key == leaf.owner_key && old.nonce == leaf.nonce
            }) {
                return Err("leaf identity already exists in the tree");
            }
        }
        let mut changes = Vec::with_capacity(self.depth as usize + 1);
        let mut node = leaf.as_ref().map(Leaf::node).unwrap_or_default();
        let mut index = position;
        changes.push((0, index, node));
        for height in 0..self.depth {
            let sibling = self.subtree(height, index ^ 1)?;
            node = if index & 1 == 0 {
                Node::parent(node, sibling)?
            } else {
                Node::parent(sibling, node)?
            };
            index >>= 1;
            changes.push((height + 1, index, node));
        }
        if let Some(leaf) = leaf {
            self.leaves.insert(position, leaf);
        } else {
            self.leaves.remove(&position);
        }
        for (height, index, node) in changes {
            if node == Node::empty(height) {
                self.nodes.remove(&(height, index));
            } else {
                self.nodes.insert((height, index), node);
            }
        }
        Ok(())
    }

    /// Prove an occupied or empty leaf position against the current root.
    pub fn proof(&self, position: u32) -> Result<Proof, &'static str> {
        self.proof_subtree(0, position)
    }

    /// Prove an aligned subtree against the current root.
    pub fn proof_subtree(&self, height: u8, mut index: u32) -> Result<Proof, &'static str> {
        self.subtree(height, index)?;
        let mut path = Vec::with_capacity((self.depth - height) as usize);
        for h in height..self.depth {
            path.push(Sibling {
                right: index & 1 != 0,
                node: self.subtree(h, index ^ 1)?,
            });
            index >>= 1;
        }
        Ok(Proof(path))
    }
}

/// Derive the public state scalar from the committed root hash.
pub fn state_tweak(root: [u8; 32]) -> [u8; 32] {
    tagged_hash("LayerTree/state", &root)
}

/// Derive H + H_state(root)G, retaining no spendable key-path secret.
pub fn state_internal_key(root: [u8; 32]) -> Result<XOnlyPublicKey, &'static str> {
    let bytes = state_tweak(root);
    let scalar =
        bitcoin::secp256k1::Scalar::from_be_bytes(bytes).map_err(|_| "invalid state scalar")?;
    if scalar == bitcoin::secp256k1::Scalar::ZERO {
        return Err("invalid zero state scalar");
    }
    let nums = XOnlyPublicKey::from_slice(&NUMS).expect("BIP341 NUMS point is valid");
    nums.add_tweak(&Secp256k1::verification_only(), &scalar)
        .map(|(key, _)| key)
        .map_err(|_| "state point is infinity")
}

/// Bind the state root and complete transition taptree to an output key.
pub fn tree_spk(root: [u8; 32], merkle_root: [u8; 32]) -> Result<ScriptBuf, &'static str> {
    let key = state_internal_key(root)?;
    let root = bitcoin::taproot::TapNodeHash::from_byte_array(merkle_root);
    Ok(ScriptBuf::new_p2tr(
        &Secp256k1::verification_only(),
        key,
        Some(root),
    ))
}

/// Derive a unique lineage identifier from the creation input's outpoint.
pub fn tree_id(outpoint: bitcoin::OutPoint) -> [u8; 32] {
    let mut bytes = [0; 36];
    bytes[..32].copy_from_slice(&outpoint.txid.to_byte_array());
    bytes[32..].copy_from_slice(&outpoint.vout.to_le_bytes());
    tagged_hash("LayerTree/tree-id", &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::hex::{DisplayHex, FromHex};

    #[test]
    fn upstream_records_roots_proofs_and_taproot_binding() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("upstream_vectors.json")).unwrap();
        let vectors = &fixture["vectors"];
        for (height, expected) in vectors["empty_nodes"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let node = Node::empty(height as u8);
            assert_eq!(
                node.hash.to_lower_hex_string(),
                expected["hash"].as_str().unwrap()
            );
            assert_eq!(node.sum, 0);
        }
        let example = &vectors["example_tree"];
        let mut tree = Tree::new(4).unwrap();
        for (position, expected) in example["leaves"].as_object().unwrap() {
            let record: Leaf = serde_json::from_value(expected.clone()).unwrap();
            assert_eq!(
                record.serialize().to_lower_hex_string(),
                expected["serialized"].as_str().unwrap()
            );
            assert_eq!(
                record.node().hash.to_lower_hex_string(),
                expected["leaf_hash"].as_str().unwrap()
            );
            tree.set(position.parse().unwrap(), Some(record)).unwrap();
        }
        assert_eq!(
            tree.root().hash.to_lower_hex_string(),
            example["root"]["hash"].as_str().unwrap()
        );
        assert_eq!(tree.root().sum, example["root"]["sum"].as_u64().unwrap());
        for (position, record) in tree.leaves() {
            let proof = tree.proof(position).unwrap();
            assert_eq!(proof.fold(record.node()).unwrap(), tree.root());
            for (sibling, expected) in proof
                .0
                .iter()
                .zip(example["paths"][position.to_string()].as_array().unwrap())
            {
                assert_eq!(sibling.right, expected["dir"].as_u64().unwrap() == 1);
                assert_eq!(
                    sibling.node.serialize().to_lower_hex_string(),
                    expected["sc"].as_str().unwrap()
                );
            }
        }
        let m = <[u8; 32]>::from_hex(example["merkle_root_placeholder"].as_str().unwrap()).unwrap();
        assert_eq!(
            tree_spk(tree.root().hash, m)
                .unwrap()
                .as_bytes()
                .to_lower_hex_string(),
            example["scriptPubKey"].as_str().unwrap()
        );
        let output = &fixture["tree_spk_vector"];
        let r = <[u8; 32]>::from_hex(output["root_hash"].as_str().unwrap()).unwrap();
        let m = <[u8; 32]>::from_hex(output["merkle_root"].as_str().unwrap()).unwrap();
        assert_eq!(
            tree_spk(r, m).unwrap().as_bytes().to_lower_hex_string(),
            output["scriptPubKey"].as_str().unwrap()
        );
        let txid = bitcoin::Txid::from_byte_array(
            <[u8; 32]>::from_hex(example["creation_outpoint"]["txid"].as_str().unwrap()).unwrap(),
        );
        assert_eq!(
            tree_id(bitcoin::OutPoint { txid, vout: 0 }).to_lower_hex_string(),
            example["tree_id"].as_str().unwrap()
        );
    }

    #[test]
    fn failed_tree_updates_preserve_root_and_unique_identities() {
        let key = XOnlyPublicKey::from_slice(&NUMS).unwrap();
        let record = Leaf {
            amount: MAX_SUM,
            exit_key: key,
            owner_key: key,
            evictor_key: key,
            evict_after: 0,
            nonce: 1,
            seq: 0,
        };
        let mut tree = Tree::new(4).unwrap();
        tree.set(0, Some(record.clone())).unwrap();
        let root = tree.root();
        assert!(tree.set(1, Some(record.clone())).is_err());
        let mut second = record;
        second.nonce = 2;
        second.amount = 1;
        assert!(tree.set(1, Some(second)).is_err());
        assert_eq!(tree.root(), root);
        assert!(tree.leaf(1).is_none());
    }
}
