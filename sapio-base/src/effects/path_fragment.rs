// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Path  Fragments
use crate::reverse_path::ReversePath;
use crate::serialization_helpers::SArc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use std::convert::TryFrom;
use std::str::FromStr;
use std::sync::Arc;

#[derive(
    Serialize, Deserialize, Debug, Hash, Eq, PartialEq, JsonSchema, Clone, PartialOrd, Ord,
)]
#[serde(into = "String")]
#[serde(try_from = "&str")]
pub enum PathFragment {
    Root,
    Cloned,
    Action,
    FinishFn,
    CondCompIf,
    Guard,
    Next,
    Suggested,
    DefaultEffect,
    Effects,
    Metadata,
    Branch(u64),
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

#[derive(Serialize, Deserialize, Debug, Hash, Eq, PartialEq, JsonSchema, Clone)]
pub enum ValidFragmentError {
    BranchParseError,
    BadName(SArc<String>),
    InvalidReversePath(&'static str),
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

impl From<ReversePath<PathFragment>> for String {
    fn from(r: ReversePath<PathFragment>) -> String {
        let mut v: Vec<String> = r.iter().cloned().map(String::from).collect();
        v.reverse();
        v.join("/")
    }
}

impl TryFrom<&str> for ReversePath<PathFragment> {
    type Error = ValidFragmentError;
    fn try_from(r: &str) -> Result<ReversePath<PathFragment>, Self::Error> {
        let frags = r
            .split('/')
            .map(PathFragment::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        ReversePath::try_from(frags).map_err(ValidFragmentError::InvalidReversePath)
    }
}

impl TryFrom<String> for ReversePath<PathFragment> {
    type Error = ValidFragmentError;
    fn try_from(r: String) -> Result<ReversePath<PathFragment>, Self::Error> {
        Self::try_from(r.as_ref())
    }
}
