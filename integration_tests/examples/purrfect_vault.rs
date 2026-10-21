//! Execute both vault routes with synthetic coins; never broadcasts.

use sapio_integration_tests::purrfect_vault_example::{scenario, Error};

fn main() -> Result<(), Error> {
    println!("{}", serde_json::to_string_pretty(&scenario(None)?)?);
    Ok(())
}
