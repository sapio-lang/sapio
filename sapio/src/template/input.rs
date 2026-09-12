// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Metadata attached to a template input.
use super::*;
use sapio_base::simp::SIMPError;
use serde::{Deserialize, Serialize};
use std::collections::btree_map::Entry;
/// Metadata for inputs, including arbitrary key-value annotations.
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

    /// Add a SIMP to the input metadata without replacing an existing value.
    ///
    /// Returns [`SIMPError::AlreadyDefined`] if one was previously set.
    /// Failed insertion leaves the metadata unchanged.
    pub fn add_simp_inplace<S: SIMPAttachableAt<TemplateInputLT>>(
        &mut self,
        s: S,
    ) -> Result<(), SIMPError> {
        match self.simp.entry(s.get_protocol_number()) {
            Entry::Occupied(entry) => Err(SIMPError::AlreadyDefined(entry.get().clone())),
            Entry::Vacant(entry) => {
                entry.insert(s.to_json()?);
                Ok(())
            }
        }
    }
    /// Add a SIMP to the input metadata without replacing an existing value.
    ///
    /// Returns [`SIMPError::AlreadyDefined`] if one was previously set.
    pub fn add_simp<S: SIMPAttachableAt<TemplateInputLT>>(
        mut self,
        s: S,
    ) -> Result<Self, SIMPError> {
        self.add_simp_inplace(s)?;
        Ok(self)
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

#[cfg(test)]
mod tests {
    use super::*;
    use sapio_base::simp::SIMP;
    use serde_json::{json, Value};

    struct Annotation<const PROTOCOL: i64>(Option<Value>);

    impl<const PROTOCOL: i64> SIMP for Annotation<PROTOCOL> {
        fn static_get_protocol_number() -> i64 {
            PROTOCOL
        }

        fn get_protocol_number(&self) -> i64 {
            PROTOCOL
        }

        fn to_json(&self) -> Result<Value, serde_json::Error> {
            self.0.clone().ok_or_else(|| {
                serde_json::Error::io(std::io::Error::other("annotation cannot serialize"))
            })
        }

        fn from_json(value: Value) -> Result<Self, serde_json::Error> {
            Ok(Self(Some(value)))
        }
    }

    impl<const PROTOCOL: i64> SIMPAttachableAt<TemplateInputLT> for Annotation<PROTOCOL> {}

    #[test]
    fn failed_simp_insertions_preserve_input_metadata() {
        let original = json!({"signer": "original"});
        let mut metadata = InputMetadata::from([("label", json!("funding"))])
            .add_simp(Annotation::<-17>(Some(original.clone())))
            .unwrap();
        let before = metadata.clone();

        let duplicate = metadata.add_simp_inplace(Annotation::<-17>(Some(json!({
            "signer": "replacement"
        }))));
        assert!(matches!(duplicate, Err(SIMPError::AlreadyDefined(value)) if value == original));
        assert_eq!(metadata, before);

        let invalid = metadata.add_simp_inplace(Annotation::<-18>(None));
        assert!(matches!(invalid, Err(SIMPError::SerializationError(_))));
        assert_eq!(metadata, before);
    }
}
