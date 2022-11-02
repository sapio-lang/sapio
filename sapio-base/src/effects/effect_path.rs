// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Module defining EffectPath, which is best understood as an execution trace through the lifetime of a Sapio contract
use std::{convert::TryFrom, iter::FromIterator, sync::Arc};

use crate::effects::path_fragment::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// All of the way that EffectPaths work internally hinge on this structure which is a trivial implementation of
// a linked list of PathFragments. As such, having many copies of "slightly different" traces is comparatively
// cheap on memory. This is useful in situations where you want to do simultaneous exploration of different
// execution paths.
//
// Semantically, when we inductively destructure this value we are always looking at the end of the trace. This
// makes it easy to have many different branching futures, but not many converging pasts. In practice we only really
// want the branching futures anyway. As a result of this, the "beginning" of the trace (chronologically) is at the
// "end" of the linked list, when you traverse it naturally. Users of this API will not have to deal with this as
// the IntoIterator implementation of EffectPath takes care of this reversal but will be helpful to understand the code
// below
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum EffectPathInner {
    /// The beginning of the effect trace
    EffectRoot,
    /// Inductive step of appending an PathFragment to an EffectPath
    EffectCons(PathFragment, EffectPath),
}

/// Structure for describing execution traces through a contract's lifetime.
/// The idea is that we should be able to get reproducible output from the contract
/// by indexing into it with this value. It is implemented using persistent data
/// structures and so Clones should be considered very cheap.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(into = "String")]
#[serde(try_from = "&str")]
pub struct EffectPath(Arc<EffectPathInner>);
impl EffectPath {
    /// Constructs a new empty EffectPath
    pub fn new() -> Self {
        EffectPath(Arc::new(EffectPathInner::EffectRoot))
    }
    /// Adds an effect to the end of an effect path
    pub fn push(&self, frag: PathFragment) -> Self {
        EffectPath(Arc::new(EffectPathInner::EffectCons(frag, self.clone())))
    }
}

impl JsonSchema for EffectPath {
    fn schema_name() -> String {
        String::from("EffectPath")
    }

    fn json_schema(gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        String::json_schema(gen)
    }
}

// Iterator stuff
impl<'a> IntoIterator for &'a EffectPath {
    type Item = &'a PathFragment;

    type IntoIter = std::iter::Rev<std::vec::IntoIter<&'a PathFragment>>;

    fn into_iter(self) -> Self::IntoIter {
        let mut ep = &*self.0;
        let mut acc = Vec::new();
        while let EffectPathInner::EffectCons(last, init) = ep {
            acc.push(last);
            ep = &*init.0;
        }
        acc.into_iter().rev()
    }
}

impl FromIterator<PathFragment> for EffectPath {
    fn from_iter<T: IntoIterator<Item = PathFragment>>(iter: T) -> Self {
        let mut ep = EffectPath::new();
        for frag in iter {
            ep = ep.push(frag);
        }
        ep
    }
}

// Conversions
impl From<PathFragment> for EffectPath {
    fn from(frag: PathFragment) -> Self {
        EffectPath(Arc::new(EffectPathInner::EffectCons(
            frag,
            EffectPath::new(),
        )))
    }
}

impl From<Vec<PathFragment>> for EffectPath {
    fn from(value: Vec<PathFragment>) -> Self {
        let mut ep = EffectPath(Arc::new(EffectPathInner::EffectRoot));
        for pf in value {
            ep = EffectPath(Arc::new(EffectPathInner::EffectCons(pf, ep)))
        }
        ep
    }
}

impl From<EffectPath> for String {
    fn from(path: EffectPath) -> String {
        let mut out = String::new();
        let fragments: Vec<String> = path.into_iter().map(|pf| String::from(&*pf)).collect();
        for s in &fragments {
            out.reserve(s.len() + 1)
        }
        let mut fragment_iter = fragments.into_iter();
        match fragment_iter.next() {
            None => out,
            Some(head) => {
                out.push_str(&head);
                for s in fragment_iter {
                    out.push('/');
                    out.push_str(&s);
                }
                out
            }
        }
    }
}

impl TryFrom<&str> for EffectPath {
    type Error = ValidFragmentError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        let res = s
            .split('/')
            .map(PathFragment::try_from)
            .collect::<Result<EffectPath, ValidFragmentError>>()?;
        Ok(res)
    }
}

impl TryFrom<String> for EffectPath {
    type Error = ValidFragmentError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        EffectPath::try_from(&*s)
    }
}
