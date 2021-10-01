// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! general non-parameter compilation state required by all contracts
use super::interned_strings::get_interned;
use super::{Amount, Compilable, CompilationError, Compiled};
use crate::contract::compiler::InternalCompilerTag;
use crate::contract::interned_strings::CLONED;
use crate::util::amountrange::AmountRange;
use crate::util::reverse_path::{MkReversePath, ReversePath};
use bitcoin::Network;
use miniscript::Descriptor;
use miniscript::DescriptorTrait;
use sapio_ctv_emulator_trait::CTVEmulator;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use std::ops::Deref;
use std::sync::Arc;

/// Error types for EffectDB Accesses
#[derive(Debug)]
pub enum EffectDBError {
    /// Error was from Deserialization
    SerializationError(serde_json::Error),
    /// Missing effect error
    NoEffectError(Arc<ReversePath<String>>),
}

/// A Generic Trait for EffectDB Functionality
pub trait EffectDB {
    /// internal implementation to retrieve a JSON for the path
    fn get_value_impl(
        &self,
        at: &Arc<ReversePath<String>>,
    ) -> Result<&Vec<serde_json::Value>, EffectDBError>;
    /// intended to be used function which casts into a native type
    /// can be overriden to directly get native type.
    fn get_value<T>(&self, at: &Arc<ReversePath<String>>) -> Result<Vec<T>, EffectDBError>
    where
        Self: Sized,
        T: for<'de> serde::Deserialize<'de>,
    {
        Ok(self
            .get_value_impl(at)?
            .iter()
            .cloned()
            .map(serde_json::from_value)
            .collect::<Result<Vec<_>, _>>()?)
    }
}
/// # A Registry of all Effects to process during compilation.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct MapEffectDB {
    /// # The set of all effects
    effects: HashMap<Arc<ReversePath<String>>, Vec<serde_json::Value>>,
}

impl EffectDB for MapEffectDB {
    fn get_value_impl(
        &self,
        at: &Arc<ReversePath<String>>,
    ) -> Result<&Vec<serde_json::Value>, EffectDBError> {
        self.effects
            .get(at)
            .ok_or_else(|| EffectDBError::NoEffectError(at.clone()))
    }
}
