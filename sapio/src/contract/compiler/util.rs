// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! utility functions for compiler

use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::Hash;
use bitcoin::XOnlyPublicKey;
use miniscript::descriptor::TapTree;
use miniscript::*;
use sapio_base::miniscript;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::sync::Arc;

/// A reproducible internal point with no known discrete logarithm. Retain its
/// original derivation so script-only outputs do not change unnecessarily.
fn unspendable_internal_key() -> XOnlyPublicKey {
    XOnlyPublicKey::from_slice(&Sha256::hash(&[1u8; 32]).into_inner())
        .expect("fixed hash is a valid x-only point")
}

/// Promote only an independently sufficient, bare key predicate. Choosing the
/// smallest eligible key makes the output independent of alternative order.
/// Constrained keys must remain behind their scripts, including inscriptions.
pub fn pick_key_from_miniscripts<'a, I: Iterator<Item = &'a Miniscript<XOnlyPublicKey, Tap>>>(
    branches: I,
) -> XOnlyPublicKey {
    branches
        .filter_map(|f| {
            if let Terminal::Check(check) = &f.node {
                if let Terminal::PkK(k) = &check.node {
                    return Some(*k);
                }
            }
            None
        })
        .min()
        .unwrap_or_else(unspendable_internal_key)
}

/// Build a deterministic tree of distinct script alternatives. Deduplication is
/// by complete script bytes: repeated envelopes within one script survive, and
/// different envelope contents or ordering remain different alternatives.
pub fn branches_to_tree(
    branches: Vec<Miniscript<XOnlyPublicKey, Tap>>,
) -> Option<TapTree<XOnlyPublicKey>> {
    let mut distinct_scripts = BTreeMap::new();
    for branch in branches {
        match distinct_scripts.entry(branch.encode()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(branch);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                // Different ASTs can encode the same script. Keep a stable
                // representative because AST ordering also breaks heap ties.
                if branch < *entry.get() {
                    entry.insert(branch);
                }
            }
        }
    }
    let mut scripts: BinaryHeap<(Reverse<u64>, TapTree<XOnlyPublicKey>)> = distinct_scripts
        .into_iter()
        .map(|(_, branch)| (Reverse(1), TapTree::Leaf(Arc::new(branch))))
        .collect();
    while scripts.len() > 1 {
        let (w1, v1) = scripts.pop().unwrap();
        let (w2, v2) = scripts.pop().unwrap();
        scripts.push((
            Reverse(w1.0.saturating_add(w2.0)),
            TapTree::Tree(Arc::new(v1), Arc::new(v2)),
        ));
    }
    scripts.pop().map(|v| v.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use miniscript::ord::Inscription;
    use std::str::FromStr;

    #[test]
    fn byte_identical_inscription_asts_have_one_stable_leaf() {
        let key = unspendable_internal_key();
        let envelope = Inscription::new(None, Some(b"same script".to_vec()));
        let outside = Miniscript::<XOnlyPublicKey, Tap>::from_str(&format!(
            "inscribe_pre({envelope},pk({key}))"
        ))
        .unwrap();
        let inside = Miniscript::<XOnlyPublicKey, Tap>::from_str(&format!(
            "c:inscribe_pre({envelope},pk_k({key}))"
        ))
        .unwrap();
        assert_ne!(outside, inside);
        assert_eq!(outside.encode(), inside.encode());
        let forward = branches_to_tree(vec![outside.clone(), inside.clone()]).unwrap();
        let reversed = branches_to_tree(vec![inside, outside]).unwrap();
        assert_eq!(forward, reversed);
        assert_eq!(forward.iter().count(), 1);
    }
}
