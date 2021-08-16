use bitcoin::*;
use jsonschema::JSONSchema;
use sapio_wasm_plugin::*;
use schemars::*;
use serde::*;
use std::error::Error;
pub trait APIHandle {
    fn get_api(&self) -> serde_json::Value;
}
pub trait SapioJSONTrait: JsonSchema + Serialize + for<'a> Deserialize<'a> {
    fn get_example_for_api_checking() -> Self;
    fn check_trait_implemented_inner(&self, api: &dyn APIHandle) -> Result<(), Box<dyn Error>> {
        let tag = Self::get_example_for_api_checking();
        let japi = api.get_api();
        let compiled = JSONSchema::compile(&japi).map_err(|_| "Error Compiling Schema")?;
        compiled
            .validate(&serde_json::to_value(CreateArgs {
                arguments: tag,
                amount: Amount::from_sat(0),
                network: Network::Bitcoin,
            })?)
            .map_err(|_| String::from("Validation Error"))?;
        Ok(())
    }
    fn check_trait_implemented(&self, api: &dyn APIHandle) -> bool {
        self.check_trait_implemented_inner(api).is_ok()
    }
}

