// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! general non-parameter compilation state required by all contracts


use crate::contract::abi::continuation::rs::SArc;



use crate::util::reverse_path::{ReversePath};





use serde::{Deserialize, Serialize};
use std::collections::HashMap;


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
    effects: HashMap<SArc<ReversePath<String>>, HashMap<SArc<String>, serde_json::Value>>,
    empty: HashMap<SArc<String>, serde_json::Value>,
}

impl EffectDB for MapEffectDB {
    fn get_value<'a>(
        &'a self,
        at: &Arc<ReversePath<String>>,
    ) -> Box<dyn Iterator<Item = (&'a Arc<String>, &'a serde_json::Value)> + 'a> {
        Box::new(
            self.effects
                .get(&SArc(at.clone()))
                .unwrap_or(&self.empty)
                .iter()
                .map(|(a, b)| (&a.0, b)),
        )
    }
}
