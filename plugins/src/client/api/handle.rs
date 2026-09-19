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
use schemars::generate::Contract;
use std::marker::PhantomData;

/// A resolved module key with typed call arguments and results.
///
/// Construction resolves the locator. Each typed call supplies its expected
/// argument/result schemas for comparison with the running module's API before
/// execution. Actual values are also validated against the advertised schemas.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
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

impl<T, R> JsonSchema for SapioHostAPI<T, R>
where
    T: Serialize + JsonSchema + Clone,
    R: for<'a> Deserialize<'a> + JsonSchema,
{
    fn schema_name() -> std::borrow::Cow<'static, str> {
        format!("Module_{}_to_{}", T::schema_name(), R::schema_name()).into()
    }

    fn schema_id() -> std::borrow::Cow<'static, str> {
        format!("SapioHostAPI<{},{}>", T::schema_id(), R::schema_id()).into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let mut schema = SapioHostAPIResolver::json_schema(generator);
        schema.insert(
            "x-sapio-module".into(),
            serde_json::json!({
                "arguments": generator
                    .subschema_for_with_contract::<CreateArgs<T>>(Contract::Deserialize),
                "returns": generator.subschema_for_with_contract::<R>(Contract::Serialize),
            }),
        );
        schema
    }
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
        super::util::call_path_typed(path, &self.key, c.clone())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Serialize, JsonSchema)]
    struct Arguments {
        #[schemars(schema_with = "sapio_base::schema::satoshis")]
        amount: u64,
    }

    #[derive(JsonSchema)]
    #[serde(transparent)]
    #[allow(dead_code)]
    struct RegistrationWrapper(Arguments);

    #[test]
    fn typed_module_schema_exports_both_generic_signatures() {
        let schema = schemars::generate::SchemaSettings::draft07()
            .into_generator()
            .into_root_schema_for::<SapioHostAPI<Arguments, u64>>();
        let marker = &schema.as_value()["x-sapio-module"];
        let actual = API::<CreateArgs<RegistrationWrapper>, u64>::new();
        assert!(crate::interface::schema_nodes_match(
            &marker["arguments"],
            schema.as_value(),
            actual.input().as_value(),
            actual.input().as_value(),
        ));
        assert!(crate::interface::schema_nodes_match(
            &marker["returns"],
            schema.as_value(),
            actual.output().as_value(),
            actual.output().as_value(),
        ));
        let wrong = API::<(), String>::new();
        assert!(!crate::interface::schema_nodes_match(
            &marker["returns"],
            schema.as_value(),
            wrong.output().as_value(),
            wrong.output().as_value(),
        ));
        assert!(schema.as_value()["properties"]["which_plugin"].is_object());
    }

    #[derive(Clone, Serialize, Deserialize, JsonSchema)]
    struct RecursiveArguments {
        #[serde(rename(serialize = "sent", deserialize = "accepted"))]
        value: u64,
        child: Option<SapioHostAPI<RecursiveArguments, RecursiveResult>>,
    }

    #[derive(Clone, Serialize, Deserialize, JsonSchema)]
    struct RecursiveResult {
        #[serde(rename(serialize = "produced", deserialize = "received"))]
        value: u64,
        next: Option<SapioHostAPI<RecursiveArguments, RecursiveResult>>,
    }

    #[test]
    fn recursive_callable_signatures_share_graphs_and_keep_each_direction() {
        let api = API::<CreateArgs<RecursiveArguments>, RecursiveResult>::new();
        for root in [api.input().as_value(), api.output().as_value()] {
            let definitions = root["definitions"].as_object().unwrap();
            let marker = definitions
                .values()
                .find_map(|value| value.get("x-sapio-module"))
                .expect("recursive handle definition");
            for (side, actual) in [
                ("arguments", api.input().as_value()),
                ("returns", api.output().as_value()),
            ] {
                assert!(crate::interface::schema_nodes_match(
                    &marker[side],
                    root,
                    actual,
                    actual,
                ));
            }
        }
        assert!(api.output().as_value()["properties"]["produced"].is_object());
        assert!(
            api.input().as_value()["definitions"]["RecursiveArguments"]["properties"]["accepted"]
                .is_object()
        );
    }
}
