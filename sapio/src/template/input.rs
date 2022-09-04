// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Template Output container
use super::*;
use sapio_base::simp::SIMPError;
use serde::{Deserialize, Serialize};
/// Metadata for outputs, arbitrary KV set.
#[derive(Serialize, Deserialize, Clone, JsonSchema, Debug, PartialEq, Eq)]
pub struct InputMetadata {
    /// Additional non-standard fields for future upgrades
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
    /// SIMP: Sapio Interactive Metadata Protocol
    pub simp: BTreeMap<i64, serde_json::Value>,
}

impl InputMetadata {
    /// Is there any metadata in this field?
    pub fn is_empty(&self) -> bool {
        *self == Default::default()
    }

    /// attempts to add a SIMP to the input metadata.
    ///
    /// Returns [`SIMPError::AlreadyDefined`] if one was previously set.
    pub fn add_simp_inplace<S: SIMPAttachableAt<TemplateInputLT>>(
        &mut self,
        s: S,
    ) -> Result<(), SIMPError> {
        let old = self.simp.insert(s.get_protocol_number(), s.to_json()?);
        if let Some(old) = old {
            Err(SIMPError::AlreadyDefined(old))
        } else {
            Ok(())
        }
    }
    /// attempts to add a SIMP to the input metadata.
    ///
    /// Returns [`SIMPError::AlreadyDefined`] if one was previously set.
    pub fn add_simp<S: SIMPAttachableAt<TemplateInputLT>>(
        mut self,
        s: S,
    ) -> Result<Self, SIMPError> {
        let old = self.simp.insert(s.get_protocol_number(), s.to_json()?);
        if let Some(old) = old {
            Err(SIMPError::AlreadyDefined(old))
        } else {
            Ok(self)
        }
    }
}
impl Default for InputMetadata {
    fn default() -> Self {
        InputMetadata {
            extra: Default::default(),
            simp: Default::default(),
        }
    }
}

impl<const N: usize> From<[(&str, serde_json::Value); N]> for InputMetadata {
    fn from(v: [(&str, serde_json::Value); N]) -> InputMetadata {
        InputMetadata {
            extra: IntoIterator::into_iter(v)
                .map(|(a, b)| (a.into(), b))
                .collect(),
            simp: Default::default(),
        }
    }
}