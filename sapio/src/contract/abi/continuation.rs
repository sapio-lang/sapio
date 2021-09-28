// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! ABI for contract resumption

use schemars::schema::RootSchema;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
/// Instructions for how to resume a contract compilation at a given point
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ContinuationPoint {
    /// The arguments required at this point
    /// TODO: De-Duplicate repeated types?
    #[serde(serialize_with = "rs::serializer")]
    #[serde(deserialize_with = "rs::deserializer")]
    pub schema: Arc<RootSchema>,
}

mod rs {
    use schemars::schema::RootSchema;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::borrow::Borrow;
    use std::sync::Arc;
    pub fn serializer<S>(v: &Arc<RootSchema>, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let b: &RootSchema = v.borrow();
        b.serialize(s)
    }
    pub fn deserializer<'de, D>(d: D) -> Result<Arc<RootSchema>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Arc::new(RootSchema::deserialize(d)?))
    }
}
