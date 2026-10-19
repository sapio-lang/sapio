//! Rebind the same complete vault flow to the isolated node's real outpoints.

use sapio_integration_tests::purrfect_vault_example::{scenario, Error, Funding};

fn main() -> Result<(), Error> {
    let supplied: Option<Funding> = std::env::args_os()
        .nth(1)
        .map(|path| -> Result<Funding, Error> {
            Ok(serde_json::from_slice(&std::fs::read(path)?)?)
        })
        .transpose()?;
    println!("{}", serde_json::to_string_pretty(&scenario(supplied)?)?);
    Ok(())
}
