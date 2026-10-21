//! Run the signed LayerTree lifecycle with disposable offline coins.
fn main() -> Result<(), sapio_integration_tests::layertree_example::Error> {
    println!(
        "{}",
        serde_json::to_string_pretty(&sapio_integration_tests::layertree_example::scenario()?)?
    );
    Ok(())
}
