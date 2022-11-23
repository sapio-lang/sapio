use sapio_data_repr::{Repr, ReprSpec, ReprSpecifiable};
use serde::*;

pub trait SapioAPIHandle {
    fn get_api(&self) -> ReprSpec;
}
impl SapioAPIHandle for ReprSpec {
    fn get_api(&self) -> Self {
        self.clone()
    }
}
pub trait SapioSchemaValidatable: ReprSpecifiable + Serialize + for<'a> Deserialize<'a> {
    fn get_example_for_api_checking() -> Repr;
    fn check_trait_implemented_inner(api: &dyn SapioAPIHandle) -> Result<(), String> {
        let tag = Self::get_example_for_api_checking();
        let japi = api.get_api();
        // let compiled = JSONSchema::compile(&japi).map_err(|_| "Error Compiling Schema")?;
        // compiled
        //     .validate(
        //         &serde_json::to_value(CreateArgs {
        //             arguments: tag,
        //             context: ContextualArguments {
        //                 amount: Amount::from_sat(0),
        //                 network: Network::Bitcoin,
        //                 effects: Default::default(),
        //             },
        //         })
        //         .map_err(|e| format!("{:?}", e))?,
        //     )
        //     .map_err(|e| {
        //         let mut s = String::from("Validation Errors:");
        //         for error in e {
        //             s += &format!("\n    - {}", error);
        //         }
        //         s
        //     })?;
        Ok(())
    }
    fn check_trait_implemented(api: &dyn SapioAPIHandle) -> bool {
        Self::check_trait_implemented_inner(api).is_ok()
    }
}
