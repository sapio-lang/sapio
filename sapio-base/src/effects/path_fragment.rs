// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Path  Fragments
use crate::serialization_helpers::SArc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use std::convert::TryFrom;
use std::str::FromStr;
use std::sync::Arc;

/// The derivation path fragments allowed, including user-generated
#[derive(
    Serialize, Deserialize, Debug, Hash, Eq, PartialEq, JsonSchema, Clone, PartialOrd, Ord,
)]
#[serde(into = "String")]
#[serde(try_from = "&str")]
pub enum PathFragment {
    /// The start of a compilation process
    Root,
    /// A Clone of some executing context
    Cloned,
    /// An Action Branch
    Action,
    /// A Finish Function
    FinishFn,
    /// A Conditional Compilation Guard
    CondCompIf,
    /// A (Clause Generating) Guard
    Guard,
    /// A Next Clause
    Next,
    /// Suggested Transaction
    Suggested,
    /// The Default Effect passed into a Continuation
    DefaultEffect,
    /// All the Effects for a Coninuation
    Effects,
    /// Metadata Creation
    Metadata,
    /// A numbered branch at this level
    Branch(u64),
    /// a named branch at this level
    Named(SArc<String>),
}

impl From<PathFragment> for String {
    fn from(a: PathFragment) -> Self {
        Self::from(&a)
    }
}
impl From<&PathFragment> for String {
    fn from(a: &PathFragment) -> Self {
        match a {
            PathFragment::Root => "@root".into(),
            PathFragment::Cloned => "@cloned".into(),
            PathFragment::Action => "@action".into(),
            PathFragment::FinishFn => "@finish_fn".into(),
            PathFragment::CondCompIf => "@cond_comp_if".into(),
            PathFragment::Guard => "@guard".into(),
            PathFragment::Next => "@next".into(),
            PathFragment::Suggested => "@suggested".into(),
            PathFragment::DefaultEffect => "@default_effect".into(),
            PathFragment::Effects => "@effects".into(),
            PathFragment::Metadata => "@metadata".into(),
            PathFragment::Branch(u) => format!("#{}", u),
            PathFragment::Named(SArc(a)) => a.as_ref().clone(),
        }
    }
}

/// Error for parsing a fragment
#[derive(Serialize, Deserialize, Debug, Hash, Eq, PartialEq, JsonSchema, Clone)]
pub enum ValidFragmentError {
    /// branch could not be parsed (must be alphanumeric for user generated)
    BranchParseError,
    /// name was using some reserved characters
    BadName(SArc<String>),
}

impl std::error::Error for ValidFragmentError {}
impl std::fmt::Display for ValidFragmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        std::fmt::Debug::fmt(self, f)
    }
}
use std::num::ParseIntError;
impl From<ParseIntError> for ValidFragmentError {
    fn from(_u: ParseIntError) -> ValidFragmentError {
        ValidFragmentError::BranchParseError
    }
}

impl TryFrom<Arc<String>> for PathFragment {
    type Error = ValidFragmentError;
    fn try_from(s: Arc<String>) -> Result<Self, Self::Error> {
        Self::try_from(s.as_ref().as_str())
    }
}
impl TryFrom<&str> for PathFragment {
    type Error = ValidFragmentError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Ok(match s {
            "@root" => PathFragment::Root,
            "@cloned" => PathFragment::Cloned,
            "@action" => PathFragment::Action,
            "@finish_fn" => PathFragment::FinishFn,
            "@cond_comp_if" => PathFragment::CondCompIf,
            "@guard" => PathFragment::Guard,
            "@next" => PathFragment::Next,
            "@suggested" => PathFragment::Suggested,
            "@default_effect" => PathFragment::DefaultEffect,
            "@effects" => PathFragment::Effects,
            "@metadata" => PathFragment::Metadata,
            n if n.starts_with('#') => PathFragment::Branch(FromStr::from_str(&n[1..])?),
            n if n.chars().all(|x| x.is_ascii_alphanumeric() || x == '_') => {
                PathFragment::Named(SArc(Arc::new(s.into())))
            }
            _ => return Err(ValidFragmentError::BadName(SArc(Arc::new(s.into())))),
        })
    }
}
