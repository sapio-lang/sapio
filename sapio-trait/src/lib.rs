use serde_json::Value;
use bitcoin::*;
use jsonschema::JSONSchema;
use sapio_base::plugin_args::CreateArgs;
use schemars::*;
use serde::*;
use std::error::Error;
pub trait SapioAPIHandle {
    fn get_api(&self) -> serde_json::Value;
}
impl SapioAPIHandle for serde_json::Value {
    fn get_api(&self) -> Self {
        self.clone()
    }
}
pub trait SapioJSONTrait: JsonSchema + Serialize + for<'a> Deserialize<'a> {
    fn get_example_for_api_checking() -> Value;
    fn check_trait_implemented_inner(api: &dyn SapioAPIHandle) -> Result<(), Box<dyn Error>> {
        let tag = Self::get_example_for_api_checking();
        let japi = api.get_api();
        let compiled = JSONSchema::compile(&japi).map_err(|_| "Error Compiling Schema")?;
        compiled
            .validate(&serde_json::to_value(CreateArgs {
                arguments: tag,
                amount: Amount::from_sat(0),
                network: Network::Bitcoin,
            })?)
            .map_err(|e| {
                let mut s = String::from("Validation Errors:");
                for error in e {
                    s += &format!("\n    - {}", error.to_string());
                }
                s
            })?;
        Ok(())
    }
    fn check_trait_implemented(api: &dyn SapioAPIHandle) -> bool {
        Self::check_trait_implemented_inner(api).is_ok()
    }
}
