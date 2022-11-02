// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! general non-parameter compilation state required by all contracts

use crate::serialization_helpers::SArc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use std::sync::Arc;
pub mod effect_path;
pub use effect_path::*;
pub mod path_fragment;
pub use path_fragment::*;
pub mod reverse_path;
pub use reverse_path::*;

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
        at: &EffectPath,
    ) -> Box<dyn Iterator<Item = (&'a Arc<String>, &'a serde_json::Value)> + 'a>;
}
/// #  Effects
/// Map of all effects to process during compilation.  Each Key represents a
/// path, each sub-key represents the sub-path name and value.
#[derive(Clone, Default, Serialize, Deserialize, JsonSchema, Debug)]
pub struct MapEffectDB {
    /// # The set of all effects
    /// List of effects to include while compiling.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    effects: BTreeMap<EffectPath, BTreeMap<SArc<String>, serde_json::Value>>,
    #[serde(skip, default)]
    empty: BTreeMap<SArc<String>, serde_json::Value>,
}

/// # Editable Effects
/// A to/from container mirroring [`MapEffectsDB`], except the inner fields are
/// public. This allows us to edit [`MapEffectsDB`] in certain contexts where we
/// can fully replace it, without the concern of mutability for contract authors.
pub struct EditableMapEffectDB {
    /// All Effects currently in the set of effects
    pub effects: BTreeMap<EffectPath, BTreeMap<SArc<String>, serde_json::Value>>,
    /// Catch-all for extra data for future extension
    pub empty: BTreeMap<SArc<String>, serde_json::Value>,
}

impl From<MapEffectDB> for EditableMapEffectDB {
    fn from(MapEffectDB { effects, empty }: MapEffectDB) -> Self {
        Self { effects, empty }
    }
}
impl From<EditableMapEffectDB> for MapEffectDB {
    fn from(EditableMapEffectDB { effects, empty }: EditableMapEffectDB) -> Self {
        Self { effects, empty }
    }
}

impl MapEffectDB {
    /// helper function for when effectdb should not be serialized
    pub fn skip_serializing(&self) -> bool {
        self.effects.is_empty()
    }
}

impl EffectDB for MapEffectDB {
    fn get_value<'a>(
        &'a self,
        at: &EffectPath,
    ) -> Box<dyn Iterator<Item = (&'a Arc<String>, &'a serde_json::Value)> + 'a> {
        let r: &BTreeMap<_, _> = self.effects.get(at).unwrap_or(&self.empty);
        Box::new(r.iter().map(|(a, b)| (&a.0, b)))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::convert::{TryFrom, TryInto};
    #[test]
    fn test_string() {
        let v: Vec<PathFragment> = vec![
            "hello".try_into().unwrap(),
            "#123".try_into().unwrap(),
            PathFragment::FinishFn,
        ];
        let r = EffectPath::try_from(v).unwrap();
        assert_eq!(String::from(r.clone()), "hello/#123/@finish_fn");
        assert_eq!(Ok(r), EffectPath::try_from("hello/#123/@finish_fn"));
    }
    #[test]
    fn test_serde() {
        let v: Vec<PathFragment> = vec![
            "hello".try_into().unwrap(),
            PathFragment::Branch(100),
            PathFragment::FinishFn,
        ];
        let r = EffectPath::try_from(v).unwrap();
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
