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
use sapio_trait::SapioJSONTrait;
use std::marker::PhantomData;

/// A Type which represents a validated module the host can resolve and execute
/// with a given API
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq)]
#[serde(try_from = "SapioHostAPIVerifier<T, R>")]
pub struct SapioHostAPI<T: SapioJSONTrait + Clone, R: for<'a> Deserialize<'a> + JsonSchema> {
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

impl<T: SapioJSONTrait + Clone, R> PluginHandle for SapioHostAPI<T, R>
where
    R: for<'a> Deserialize<'a> + JsonSchema,
{
    type Input = CreateArgs<T>;
    type Output = R;
    fn call(&self, path: &EffectPath, c: &Self::Input) -> Result<Self::Output, CompilationError> {
        call_path(path, &self.key, c.clone())
    }
    fn get_api(&self) -> Result<API<Self::Input, Self::Output>, CompilationError> {
        get_api(&self.key)
    }
    fn get_name(&self) -> Result<String, CompilationError> {
        get_name(&self.key)
    }
    fn get_logo(&self) -> Result<String, CompilationError> {
        get_logo(&self.key)
    }
}

impl<T: SapioJSONTrait + Clone, R> SapioHostAPI<T, R>
where
    R: for<'a> Deserialize<'a> + JsonSchema,
{
    /// Ensures a [`SapioHostAPI`]'s [`LookupFrom`] field is
    /// [`LookupFrom::HashKey`] form.
    pub fn canonicalize(&self) -> Self {
        use bitcoin::hashes::hex::ToHex;
        SapioHostAPI {
            which_plugin: LookupFrom::HashKey(self.key.to_hex()),
            key: self.key,
            _pd: Default::default(),
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema)]
/// # Helper for Serialization...
struct SapioHostAPIVerifier<T: SapioJSONTrait + Clone, R: for<'a> Deserialize<'a>> {
    which_plugin: LookupFrom,
    #[serde(default, skip)]
    _pd: PhantomData<(T, R)>,
}

impl<T, R> TryFrom<LookupFrom> for SapioHostAPI<T, R>
where
    R: JsonSchema + for<'a> Deserialize<'a>,
    T: SapioJSONTrait + Clone,
{
    type Error = CompilationError;
    fn try_from(which_plugin: LookupFrom) -> Result<SapioHostAPI<T, R>, CompilationError> {
        SapioHostAPI::try_from(SapioHostAPIVerifier {
            which_plugin,
            _pd: Default::default(),
        })
    }
}
impl<T, R> TryFrom<SapioHostAPIVerifier<T, R>> for SapioHostAPI<T, R>
where
    R: schemars::JsonSchema + for<'a> Deserialize<'a>,
    T: SapioJSONTrait + Clone,
{
    type Error = CompilationError;
    fn try_from(shapv: SapioHostAPIVerifier<T, R>) -> Result<SapioHostAPI<T, R>, CompilationError> {
        let SapioHostAPIVerifier { which_plugin, _pd } = shapv;
        let key = match which_plugin.to_key() {
            Some(key) => key,
            _ => {
                return Err(CompilationError::UnknownModule);
            }
        };

        let res = SapioHostAPI {
            which_plugin,
            key,
            _pd,
        };
        let api = res.get_api()?;
        T::check_trait_implemented_inner(
            &serde_json::to_value(api.input()).map_err(CompilationError::SerializationError)?,
        )
        .map_err(CompilationError::ModuleFailedAPICheck)?;
        Ok(res)
    }
}
