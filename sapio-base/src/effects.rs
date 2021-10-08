// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! general non-parameter compilation state required by all contracts

use crate::reverse_path::ReversePath;
use crate::serialization_helpers::SArc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::str::FromStr;
use std::sync::Arc;

#[derive(Serialize, Deserialize, Debug, Hash, Eq, PartialEq, JsonSchema, Clone)]
#[serde(into = "String")]
#[serde(try_from = "&str")]
pub enum PathFragment {
    Cloned,
    ThenFn,
    FinishOrFn,
    FinishFn,
    CondCompIf,
    Guard,
    Next,
    Suggested,
    DefaultEffect,
    Effects,
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
            PathFragment::Cloned => "@cloned".into(),
            PathFragment::ThenFn => "@then_fn".into(),
            PathFragment::FinishOrFn => "@finish_or_fn".into(),
            PathFragment::FinishFn => "@finish_fn".into(),
            PathFragment::CondCompIf => "@cond_comp_if".into(),
            PathFragment::Guard => "@guard".into(),
            PathFragment::Next => "@next".into(),
            PathFragment::Suggested => "@suggested".into(),
            PathFragment::DefaultEffect => "@default_effect".into(),
            PathFragment::Effects => "@effects".into(),
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
    fn from(u: ParseIntError) -> ValidFragmentError {
        ValidFragmentError::BranchParseError
    }
}

impl TryFrom<Arc<String>> for PathFragment {
    type Error = ValidFragmentError;
    fn try_from(s: Arc<String>) -> Result<Self, Self::Error> {
        Ok(match s.as_ref().as_str() {
            "@cloned" => PathFragment::Cloned,
            "@then_fn" => PathFragment::ThenFn,
            "@finish_or_fn" => PathFragment::FinishOrFn,
            "@finish_fn" => PathFragment::FinishFn,
            "@cond_comp_if" => PathFragment::CondCompIf,
            "@guard" => PathFragment::Guard,
            "@next" => PathFragment::Next,
            "@suggested" => PathFragment::Suggested,
            "@default_effect" => PathFragment::DefaultEffect,
            "@effects" => PathFragment::Effects,
            n if n.starts_with('#') => PathFragment::Branch(FromStr::from_str(&n[1..])?),
            n if n.chars().all(|x| x.is_ascii_alphabetic() || x == '_') => {
                PathFragment::Named(SArc(s))
            }
            _ => return Err(ValidFragmentError::BadName(SArc(s.clone()))),
        })
    }
}
impl TryFrom<&str> for PathFragment {
    type Error = ValidFragmentError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Ok(match s.as_ref() {
            "@cloned" => PathFragment::Cloned,
            "@then_fn" => PathFragment::ThenFn,
            "@finish_or_fn" => PathFragment::FinishOrFn,
            "@finish_fn" => PathFragment::FinishFn,
            "@cond_comp_if" => PathFragment::CondCompIf,
            "@guard" => PathFragment::Guard,
            "@next" => PathFragment::Next,
            "@suggested" => PathFragment::Suggested,
            "@default_effect" => PathFragment::DefaultEffect,
            "@effects" => PathFragment::Effects,
            n if n.starts_with('#') => PathFragment::Branch(FromStr::from_str(&n[1..])?),
            n if n.chars().all(|x| x.is_ascii_alphabetic() || x == '_') => {
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
/// Error types for EffectDB Accesses
#[derive(Debug)]
pub enum EffectDBError {
    /// Error was from Deserialization
    SerializationError(serde_json::Error),
}

impl From<serde_json::Error> for EffectDBError {
    fn from(e: serde_json::Error) -> Self {
        EffectDBError::SerializationError(e)
    }
}
/// A Generic Trait for EffectDB Functionality
pub trait EffectDB {
    /// internal implementation to retrieve a JSON for the path
    fn get_value<'a>(
        &'a self,
        at: &Arc<ReversePath<PathFragment>>,
    ) -> Box<dyn Iterator<Item = (&'a Arc<String>, &'a serde_json::Value)> + 'a>;
}
/// #  Effects
/// Map of all effects to process during compilation.  Each Key represents a
/// path, each sub-key represents the sub-path name and value.
#[derive(Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct MapEffectDB {
    /// # The set of all effects
    /// List of effects to include while compiling.
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    effects: HashMap<SArc<ReversePath<PathFragment>>, HashMap<SArc<String>, serde_json::Value>>,
    #[serde(skip, default)]
    empty: HashMap<SArc<String>, serde_json::Value>,
}
impl MapEffectDB {
    pub fn skip_serializing(&self) -> bool {
        self.effects.is_empty()
    }
}

impl EffectDB for MapEffectDB {
    fn get_value<'a>(
        &'a self,
        at: &Arc<ReversePath<PathFragment>>,
    ) -> Box<dyn Iterator<Item = (&'a Arc<String>, &'a serde_json::Value)> + 'a> {
        let r: &HashMap<_, _> = self.effects.get(&SArc(at.clone())).unwrap_or(&self.empty);
        Box::new(r.iter().map(|(a, b)| (&a.0, b)))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::convert::TryInto;
    #[test]
    fn test_string() {
        let v: Vec<PathFragment> = vec![
            "hello".try_into().unwrap(),
            "#123".try_into().unwrap(),
            PathFragment::FinishFn,
        ];
        let r = ReversePath::try_from(v).unwrap();
        assert_eq!(String::from(r.clone()), "hello/#123/@finish_fn");
        assert_eq!(Ok(r), ReversePath::try_from("hello/#123/@finish_fn"));
    }
    #[test]
    fn test_serde() {
        let v: Vec<PathFragment> = vec![
            "hello".try_into().unwrap(),
            PathFragment::Branch(100),
            PathFragment::FinishFn,
        ];
        let r = ReversePath::<PathFragment>::try_from(v).unwrap();
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            "\"hello/#100/@finish_fn\""
        );
        assert_eq!(
            Ok(r),
            serde_json::from_str("\"hello/#100/@finish_fn\"").map_err(|_| ())
        );
    }
}
