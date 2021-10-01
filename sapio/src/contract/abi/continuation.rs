// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! ABI for contract resumption

use crate::util::reverse_path::ReversePath;
use schemars::schema::RootSchema;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
/// Instructions for how to resume a contract compilation at a given point
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ContinuationPoint {
    /// The arguments required at this point
    /// TODO: De-Duplicate repeated types?
    pub schema: Option<rs::SArc<RootSchema>>,
    /// The path at which this was compiled
    #[serde(serialize_with = "rs::serializer")]
    #[serde(deserialize_with = "rs::deserializer")]
    pub path: Arc<ReversePath<String>>,
}
impl ContinuationPoint {
    /// Creates a new continuation
    pub fn at(schema: Option<Arc<RootSchema>>, path: Arc<ReversePath<String>>) -> Self {
        ContinuationPoint {
            schema: schema.map(rs::SArc),
            path,
        }
    }
}

mod rs {
    use schemars::JsonSchema;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::borrow::Borrow;
    use std::sync::Arc;

    #[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, PartialOrd)]
    #[serde(
        bound = "T: Serialize + for<'d> Deserialize<'d> + JsonSchema + std::fmt::Debug + Clone "
    )]
    #[serde(transparent)]
    pub struct SArc<T>(
        #[serde(serialize_with = "serializer")]
        #[serde(deserialize_with = "deserializer")]
        pub Arc<T>,
    );
    pub fn serializer<T, S>(v: &Arc<T>, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        let b: &T = v.borrow();
        b.serialize(s)
    }
    pub fn deserializer<'de, T, D>(d: D) -> Result<Arc<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Ok(Arc::new(T::deserialize(d)?))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_sarc_ser() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(serde_json::to_string(&rs::SArc(Arc::new(1)))?, "1");
        Ok(())
    }

    #[test]
    fn test_continuation_point_ser() -> Result<(), Box<dyn std::error::Error>> {
        let a: ContinuationPoint = ContinuationPoint::at(
            Some(Arc::new(schemars::schema_for!(ContinuationPoint))),
            ReversePath::push(None, Arc::new("one".into())),
        );
        let b: ContinuationPoint = serde_json::from_str(&format!(
            "{{\"schema\":{},\"path\":[\"one\"]}}",
            serde_json::to_string(&schemars::schema_for!(ContinuationPoint))?
        ))?;
        assert_eq!(a, b);
        Ok(())
    }
}
