//! Review the spending request before the CLI releases signatures.

use bitcoin::{psbt::Psbt, Amount, TxOut};
use std::error::Error;
use std::io::{self, Write};

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn total(outputs: &[TxOut]) -> Result<u64, io::Error> {
    outputs.iter().try_fold(0_u64, |sum, output| {
        sum.checked_add(output.value.to_sat())
            .filter(|sum| *sum <= Amount::MAX_MONEY.to_sat())
            .ok_or_else(|| invalid("transaction amounts exceed Bitcoin's monetary range"))
    })
}

pub(crate) fn review(psbt: &Psbt, approved: bool) -> Result<(), Box<dyn Error>> {
    let prevouts = sapio_psbt::previous_outputs(psbt)?;
    let input_total = total(&prevouts)?;
    let output_total = total(&psbt.unsigned_tx.output)?;
    let fee = input_total
        .checked_sub(output_total)
        .ok_or_else(|| invalid("transaction outputs exceed its inputs"))?;
    let witness_only = psbt
        .inputs
        .iter()
        .filter(|input| input.non_witness_utxo.is_none())
        .count();

    // Stdout is reserved for the signed PSBT, including when input is piped.
    let mut stderr = io::stderr().lock();
    writeln!(stderr, "Signing review (SIGHASH_ALL):")?;
    writeln!(stderr, "  Transaction: {}", psbt.unsigned_tx.compute_txid())?;
    writeln!(
        stderr,
        "  Inputs: {} totaling {input_total} sat",
        prevouts.len()
    )?;
    for (index, output) in psbt.unsigned_tx.output.iter().enumerate() {
        writeln!(
            stderr,
            "  Output {index}: {} sat; scriptPubKey {}",
            output.value.to_sat(),
            output.script_pubkey.to_hex_string(),
        )?;
    }
    writeln!(stderr, "  Output total: {output_total} sat")?;
    writeln!(stderr, "  Absolute fee: {fee} sat")?;
    if witness_only != 0 {
        writeln!(stderr, "  {witness_only} input(s) use PSBT-supplied witness UTXOs: amounts and scripts are unverified against previous transactions.")?;
    }
    writeln!(stderr, "  Supplied full previous transactions were checked against their outpoints; chain inclusion and unspent status were not checked.")?;
    if !approved {
        return Err(invalid(
            "review the outputs and fee above, then rerun with --yes to authorize signing",
        )
        .into());
    }
    Ok(())
}
