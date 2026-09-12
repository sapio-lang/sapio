//! Preserve a complete emulated program behind a native relative timelock.

use sapio::contract::{CompilationError, Contract};
use sapio::{declare, guard};
use sapio_base::policy::ScriptPolicy;
use sapio_base::program::EmulatedProgram;
use sapio_base::timelocks::RelHeight;
use sapio_base::Clause;
use schemars::JsonSchema;
use serde::Deserialize;

#[cfg(target_arch = "wasm32")]
use sapio_wasm_plugin::{optional_logo, REGISTER};

/// The caller selects the exact program instance and public oracle.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DelayedProgram {
    /// The full public instance retained in the compiled artifact.
    program: EmulatedProgram,
    /// Blocks that must pass before this output can be spent.
    #[schemars(range(min = 1))]
    delay: u16,
}

impl DelayedProgram {
    #[guard(policy, cached)]
    fn authorized(self) -> Result<ScriptPolicy, CompilationError> {
        Ok(ScriptPolicy::And(vec![
            Clause::try_from(RelHeight::from(self.delay))?.into(),
            self.program.clone().into(),
        ]))
    }
}

impl Contract for DelayedProgram {
    declare! {non updatable}
    declare! {finish, Self::authorized}
}

#[cfg(target_arch = "wasm32")]
REGISTER![DelayedProgram];
