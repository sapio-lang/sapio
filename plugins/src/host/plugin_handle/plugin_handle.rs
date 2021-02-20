use std::error::Error;
use super::*;
pub trait PluginHandle {
    fn create(&self, c: &CreateArgs<String>) -> Result<Compiled, Box<dyn Error>>;
    fn get_api(&self) -> Result<serde_json::value::Value, Box<dyn Error>>;
}