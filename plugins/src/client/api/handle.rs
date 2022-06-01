// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

///! Handle for Sapio Plugins
use super::*;
use crate::plugin_handle::PluginHandle;
use core::convert::TryFrom;
use sapio::contract::CompilationError;
use sapio_base::effects::EffectPath;
use sapio_trait::SapioJSONTrait;
use std::marker::PhantomData;

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq)]
#[serde(try_from = "SapioHostAPIVerifier<T, R>")]
pub struct SapioHostAPI<T: SapioJSONTrait, R: for<'a> Deserialize<'a> + JsonSchema> {
    pub which_plugin: LookupFrom,
    #[serde(skip, default)]
    pub key: [u8; 32],
    #[serde(default, skip)]
    _pd: PhantomData<(T, R)>,
}

pub type ContractModule<T> = SapioHostAPI<T, Compiled>;

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
        let p = self.key.as_ptr() as i32;
        let api_buf = unsafe { sapio_v1_wasm_plugin_get_api(p) };
        if api_buf == 0 {
            return Err(CompilationError::InternalModuleError(
                "API Not Available".into(),
            ));
        }
        let cs = unsafe { CString::from_raw(api_buf as *mut c_char) };
        Ok(
            serde_json::from_slice(cs.as_bytes())
                .map_err(CompilationError::DeserializationError)?,
        )
    }
    fn get_name(&self) -> Result<String, CompilationError> {
        let p = self.key.as_ptr() as i32;
        let name_buf = unsafe { sapio_v1_wasm_plugin_get_name(p) };
        if name_buf == 0 {
            return Err(CompilationError::InternalModuleError(
                "API Not Available".into(),
            ));
        }
        let cs = unsafe { CString::from_raw(name_buf as *mut c_char) };
        Ok(
            serde_json::from_slice(cs.as_bytes())
                .map_err(CompilationError::DeserializationError)?,
        )
    }
    fn get_logo(&self) -> Result<String, CompilationError> {
        let p = self.key.as_ptr() as i32;
        let logo_buf = unsafe { sapio_v1_wasm_plugin_get_logo(p) };
        if logo_buf == 0 {
            return Err(CompilationError::InternalModuleError(
                "API Not Available".into(),
            ));
        }
        let cs = unsafe { CString::from_raw(logo_buf as *mut c_char) };
        Ok(
            serde_json::from_slice(cs.as_bytes())
                .map_err(CompilationError::DeserializationError)?,
        )
    }
}

impl<T: SapioJSONTrait, R> SapioHostAPI<T, R>
where
    R: for<'a> Deserialize<'a> + JsonSchema,
{
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
struct SapioHostAPIVerifier<T: SapioJSONTrait, R: for<'a> Deserialize<'a>> {
    which_plugin: LookupFrom,
    #[serde(default, skip)]
    _pd: PhantomData<(T, R)>,
}

impl<T: SapioJSONTrait, R: for<'a> Deserialize<'a>> TryFrom<LookupFrom> for SapioHostAPI<T, R>
where
    R: JsonSchema,
{
    type Error = CompilationError;
    fn try_from(which_plugin: LookupFrom) -> Result<SapioHostAPI<T, R>, CompilationError> {
        SapioHostAPI::try_from(SapioHostAPIVerifier {
            which_plugin,
            _pd: Default::default(),
        })
    }
}
impl<T: SapioJSONTrait, R: for<'a> Deserialize<'a>> TryFrom<SapioHostAPIVerifier<T, R>>
    for SapioHostAPI<T, R>
where
    R: schemars::JsonSchema,
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
        let p = key.as_ptr() as i32;
        let api: API<T, R> = unsafe {
            let api_buf = sapio_v1_wasm_plugin_get_api(p);
            if api_buf == 0 {
                return Err(CompilationError::InternalModuleError(
                    "API Not Available".into(),
                ));
            }
            let cs = { CString::from_raw(api_buf as *mut c_char) };
            serde_json::from_slice(cs.as_bytes()).map_err(CompilationError::DeserializationError)?
        };
        T::check_trait_implemented_inner(
            &serde_json::to_value(api.input()).map_err(CompilationError::SerializationError)?,
        )
        .map_err(CompilationError::ModuleFailedAPICheck)?;
        Ok(SapioHostAPI {
            which_plugin,
            key,
            _pd,
        })
    }
}
