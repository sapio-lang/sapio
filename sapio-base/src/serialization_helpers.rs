// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
/// Helpers for serializing Arcs
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Borrow;
use std::sync::Arc;

/// Serializable Arc Type
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, PartialOrd, Eq, Hash, Ord,
)]
#[serde(bound = "T: Serialize + for<'d> Deserialize<'d> + JsonSchema + std::fmt::Debug + Clone ")]
#[serde(transparent)]
pub struct SArc<T>(
    #[serde(serialize_with = "serializer")]
    #[serde(deserialize_with = "deserializer")]
    pub Arc<T>,
);
/// arc serializer
pub fn serializer<T, S>(v: &Arc<T>, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    let b: &T = v.borrow();
    b.serialize(s)
}
/// arc deserializer
pub fn deserializer<'de, T, D>(d: D) -> Result<Arc<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Arc::new(T::deserialize(d)?))
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_sarc_ser() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(serde_json::to_string(&SArc(Arc::new(1)))?, "1");
        Ok(())
    }
}
