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
}

/// A Generic Trait for EffectDB Functionality
pub trait EffectDB {
    /// internal implementation to retrieve a JSON for the path
    fn get_value<'a>(
        &'a self,
        at: &Arc<ReversePath<String>>,
    ) -> Box<dyn Iterator<Item = (&'a Arc<String>, &'a serde_json::Value)> + 'a>;
}
/// # A Registry of all Effects to process during compilation.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct MapEffectDB {
    /// # The set of all effects
    effects: HashMap<Arc<ReversePath<String>>, HashMap<Arc<String>, serde_json::Value>>,
    empty: HashMap<Arc<String>, serde_json::Value>,
}

impl EffectDB for MapEffectDB {
    fn get_value<'a>(
        &'a self,
        at: &Arc<ReversePath<String>>,
    ) -> Box<dyn Iterator<Item = (&'a Arc<String>, &'a serde_json::Value)> + 'a> {
        Box::new(self.effects.get(at).unwrap_or(&self.empty).iter())
    }
}
