//! Validation of the schemas advertised by a module at the host call boundary.

mod complexity;

use jsonschema::{Draft, PatternOptions, Validator};
use sapio::contract::CompilationError;
use serde::Deserialize;
use serde_json::Value;

/// One input/output contract captured before invoking guest creation code.
pub(super) struct CallSchema {
    input: Validator,
    output: Validator,
}

fn api_error(message: impl Into<String>) -> CompilationError {
    CompilationError::ModuleFailedAPICheck(message.into())
}

fn compile(schema: &Value, side: &str) -> Result<Validator, CompilationError> {
    // Plugin APIs explicitly emit Draft 7. The preflight checks declarations before
    // reference resolution; neither pass may retrieve files or network data.
    complexity::check(schema)
        .map_err(|error| api_error(format!("Invalid {side} schema: {error}")))?;
    jsonschema::options()
        .with_draft(Draft::Draft7)
        .offline()
        .with_pattern_options(PatternOptions::regex())
        .build(schema)
        .map_err(|error| api_error(format!("Invalid {side} schema: {error}")))
}

impl CallSchema {
    pub(super) fn from_json(bytes: &[u8]) -> Result<Self, CompilationError> {
        // Validate the exact advertised JSON, including integer bounds that
        // cannot be represented without loss as floating point numbers.
        #[derive(Deserialize)]
        struct AdvertisedSchemas {
            arguments: Value,
            returns: Value,
        }
        let api: AdvertisedSchemas =
            serde_json::from_slice(bytes).map_err(CompilationError::DeserializationError)?;
        Ok(Self {
            input: compile(&api.arguments, "input")?,
            output: compile(&api.returns, "output")?,
        })
    }

    pub(super) fn validate_input(&self, value: &Value) -> Result<(), CompilationError> {
        Self::validate(&self.input, value, "Input")
    }

    pub(super) fn validate_output(&self, value: &Value) -> Result<(), CompilationError> {
        Self::validate(&self.output, value, "Output")
    }

    fn validate(validator: &Validator, value: &Value, side: &str) -> Result<(), CompilationError> {
        // The boolean path avoids constructing arbitrarily large error trees
        // for nested alternatives. Error status must not depend on formatting
        // every failed branch of a schema supplied by an untrusted module.
        if validator.is_valid(value) {
            Ok(())
        } else {
            Err(api_error(format!(
                "{side} JSON does not satisfy the module's advertised schema"
            )))
        }
    }
}

#[cfg(test)]
mod tests;
