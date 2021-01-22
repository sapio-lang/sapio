use bitcoin::consensus::deserialize;
use bitcoin::util::psbt::PartiallySignedTransaction;

pub fn check_file(p: &str) -> Result<(), String> {
    std::fs::metadata(p).map_err(|_| String::from("File doesn't exist"))?;
    Ok(())
}
pub fn check_file_not(p: &str) -> Result<(), String> {
    if std::fs::metadata(p).is_ok() {
        return Err(String::from("File exists already"));
    }
    Ok(())
}

pub fn decode_psbt_file(
    a: &clap::ArgMatches,
    b: &str,
) -> Result<PartiallySignedTransaction, Box<dyn std::error::Error>> {
    let bytes = std::fs::read_to_string(a.value_of_os(b).unwrap())?;
    let bytes = base64::decode(&bytes.trim()[..])?;
    let psbt: PartiallySignedTransaction = deserialize(&bytes[..])?;
    Ok(psbt)
}