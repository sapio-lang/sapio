// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Handle for Sapio Plugins
use super::util::{get_api, get_logo, get_name};
use super::*;
use crate::plugin_handle::PluginHandle;
use core::convert::TryFrom;
use sapio::contract::CompilationError;
use sapio_base::effects::EffectPath;
use sapio_base::Clause;
use std::marker::PhantomData;

/// A resolved module key with typed call arguments and results.
///
/// Construction resolves the locator; it does not prove interface compatibility.
/// The host validates each actual input and successful output against the
/// module's advertised schemas, and the caller deserializes the result as `R`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq)]
#[serde(try_from = "SapioHostAPIResolver")]
#[serde(bound(serialize = "", deserialize = ""))]
pub struct SapioHostAPI<T: Serialize + JsonSchema + Clone, R: for<'a> Deserialize<'a> + JsonSchema>
{
    /// The module's locator
    pub which_plugin: LookupFrom,
    /// when resolved, the hash of the module
    #[serde(skip, default)]
    pub key: [u8; 32],
    #[serde(default, skip)]
    _pd: PhantomData<(T, R)>,
}

/// Convenience Label for [`SapioHostAPI<T, Compiled>`]
pub type ContractModule<T> = SapioHostAPI<T, Compiled>;
/// Convenience Label for [`SapioHostAPI<T, Clause>`]
pub type ClauseModule<T> = SapioHostAPI<T, Clause>;

impl<T: Serialize + JsonSchema + Clone, R> PluginHandle for SapioHostAPI<T, R>
where
    R: for<'a> Deserialize<'a> + JsonSchema,
{
    type Input = CreateArgs<T>;
    type Output = R;
    fn call(
        &mut self,
        path: &EffectPath,
        c: &Self::Input,
    ) -> Result<Self::Output, CompilationError> {
        call_path(path, &self.key, c.clone())
    }
    fn get_api(&mut self) -> Result<API<Self::Input, Self::Output>, CompilationError> {
        get_api(&self.key)
    }
    fn get_name(&mut self) -> Result<String, CompilationError> {
        get_name(&self.key)
    }
    fn get_logo(&mut self) -> Result<String, CompilationError> {
        get_logo(&self.key)
    }
}

impl<T: Serialize + JsonSchema + Clone, R> SapioHostAPI<T, R>
where
    R: for<'a> Deserialize<'a> + JsonSchema,
{
    /// Ensures a [`SapioHostAPI`]'s [`LookupFrom`] field is
    /// [`LookupFrom::HashKey`] form.
    pub fn canonicalize(&self) -> Self {
        use bitcoin::hex::DisplayHex;
        SapioHostAPI {
            which_plugin: LookupFrom::HashKey(self.key.to_lower_hex_string()),
            key: self.key,
            _pd: Default::default(),
        }
    }
}

/// The serialized locator, resolved into a module key during deserialization.
#[derive(Deserialize, JsonSchema)]
struct SapioHostAPIResolver {
    which_plugin: LookupFrom,
}

impl<T, R> TryFrom<LookupFrom> for SapioHostAPI<T, R>
where
    R: JsonSchema + for<'a> Deserialize<'a>,
    T: Serialize + JsonSchema + Clone,
{
    type Error = CompilationError;
    fn try_from(which_plugin: LookupFrom) -> Result<SapioHostAPI<T, R>, CompilationError> {
        SapioHostAPI::try_from(SapioHostAPIResolver { which_plugin })
    }
}
impl<T, R> TryFrom<SapioHostAPIResolver> for SapioHostAPI<T, R>
where
    R: schemars::JsonSchema + for<'a> Deserialize<'a>,
    T: Serialize + JsonSchema + Clone,
{
    type Error = CompilationError;
    fn try_from(resolver: SapioHostAPIResolver) -> Result<SapioHostAPI<T, R>, CompilationError> {
        let SapioHostAPIResolver { which_plugin } = resolver;
        let key = match which_plugin.to_key() {
            Some(key) => key,
            _ => {
                return Err(CompilationError::UnknownModule);
            }
        };

        Ok(SapioHostAPI {
            which_plugin,
            key,
            _pd: Default::default(),
        })
    }
}
