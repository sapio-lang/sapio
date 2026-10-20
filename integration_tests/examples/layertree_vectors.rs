//! Export offline transactions, or bind the isolated driver's real coins.
use sapio_integration_tests::layertree_example::{scenario_funded, Error, Funding};
fn main() -> Result<(), Error> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() > 1 {
        return Err("usage: layertree_vectors [funding.json]".into());
    }
    let funding: Option<Funding> = args
        .first()
        .map(|path| Ok::<_, Error>(serde_json::from_slice(&std::fs::read(path)?)?))
        .transpose()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&scenario_funded(funding)?)?
    );
    Ok(())
}
