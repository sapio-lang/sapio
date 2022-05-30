// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! utility functions for compiler

use ::miniscript::descriptor::TapTree;
use ::miniscript::*;
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::Hash;
use bitcoin::XOnlyPublicKey;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;
/// picks a key from an iter of miniscripts, or returns a static default key
pub fn pick_key_from_miniscripts<'a, I: Iterator<Item = &'a Miniscript<XOnlyPublicKey, Tap>>>(
    branches: I,
) -> XOnlyPublicKey {
    branches
        .filter_map(|f| {
            if let Terminal::Check(check) = &f.node {
                if let Terminal::PkK(k) = &check.node {
                    return Some(k.clone());
                }
            }
            None
        })
        .next()
        .map(|x| bitcoin::util::schnorr::UntweakedPublicKey::from(x))
        .unwrap_or(
            XOnlyPublicKey::from_slice(&Sha256::hash(&[1u8; 32]).into_inner()).expect("constant"),
        )
}

/// Convert the branches into a heap for taproot tree consumption
pub fn branches_to_tree(
    branches: Vec<Miniscript<XOnlyPublicKey, Tap>>,
) -> Option<TapTree<XOnlyPublicKey>> {
    let mut scripts: BinaryHeap<(Reverse<u64>, TapTree<XOnlyPublicKey>)> = branches
        .into_iter()
        .map(|b| (Reverse(1), TapTree::Leaf(Arc::new(b))))
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
