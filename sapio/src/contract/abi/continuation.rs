// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! ABI for contract resumption

use sapio_base::serialization_helpers::SArc;
use sapio_base::simp::{SIMPAttachableAt, SIMPError, SIMP};
use sapio_base::{effects::EffectPath, simp::ContinuationPointLT};
use schemars::schema::RootSchema;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};
/// Instructions for how to resume a contract compilation at a given point
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ContinuationPoint {
    /// The arguments required at this point
    /// TODO: De-Duplicate repeated types?
    pub schema: Option<SArc<Value>>,
    /// The path at which this was compiled
    #[serde(serialize_with = "sapio_base::serialization_helpers::serializer")]
    #[serde(deserialize_with = "sapio_base::serialization_helpers::deserializer")]
    pub path: Arc<EffectPath>,
    /// Metadata for this particular Continuation Point
    pub simp: BTreeMap<i64, Value>,
}
impl ContinuationPoint {
    /// Creates a new continuation
    pub fn at(schema: Option<Arc<Value>>, path: Arc<EffectPath>) -> Self {
        ContinuationPoint {
            schema: schema.map(SArc),
            path,
            simp: Default::default(),
        }
    }

    /// attempts to add a SIMP to the output meta.
    ///
    /// Returns [`SIMPError::AlreadyDefined`] if one was previously set.
    pub fn add_simp(
        mut self,
        s: &dyn SIMPAttachableAt<ContinuationPointLT>,
    ) -> Result<Self, SIMPError> {
        let old = self.simp.insert(s.get_protocol_number(), s.to_json()?);
        if let Some(old) = old {
            Err(SIMPError::AlreadyDefined(old))
        } else {
            Ok(self)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use sapio_base::effects::PathFragment;
    #[test]
    fn test_continuation_point_ser() -> Result<(), Box<dyn std::error::Error>> {
        let a: ContinuationPoint = ContinuationPoint::at(
            Some(Arc::new(
                serde_json::to_value(&schemars::schema_for!(ContinuationPoint)).unwrap(),
            )),
            EffectPath::push(None, PathFragment::Named(SArc(Arc::new("one".into())))),
        );
        let b: ContinuationPoint = serde_json::from_str(&format!(
            "{{\"schema\":{},\"path\":\"one\",\"simp\":{{}}}}",
            serde_json::to_string(&schemars::schema_for!(ContinuationPoint))?
        ))?;
        assert_eq!(a, b);
        Ok(())
    }
}
