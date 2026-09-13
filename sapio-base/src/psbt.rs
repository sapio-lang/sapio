//! Inspect supplied previous outputs without inventing missing funding data.

use bitcoin::{psbt::Psbt, TxOut};
use std::{collections::BTreeSet, fmt};

/// Invalid funding evidence attached to a PSBT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FundingError {
    /// Transaction and PSBT maps disagree.
    Structure(&'static str),
    /// The same outpoint is spent twice.
    DuplicateInput(usize),
    /// A previous transaction has the wrong identity or no selected output.
    PreviousTransaction(usize),
    /// Witness and non-witness previous outputs disagree.
    ConflictingOutputs(usize),
}

impl fmt::Display for FundingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Structure(reason) => write!(f, "invalid PSBT: {reason}"),
            Self::DuplicateInput(index) => write!(f, "input {index} repeats an outpoint"),
            Self::PreviousTransaction(index) => {
                write!(f, "input {index} has an invalid previous transaction")
            }
            Self::ConflictingOutputs(index) => {
                write!(f, "input {index} has conflicting previous outputs")
            }
        }
    }
}

impl std::error::Error for FundingError {}

/// Check every supplied previous transaction and return ordered funding data.
///
/// `None` means no previous output was supplied. Witness-only outputs describe
/// the amounts/scripts that signatures will commit to; they do not establish
/// chain existence, confirmation, or unspentness. No lookup or signing occurs.
pub fn previous_outputs(psbt: &Psbt) -> Result<Vec<Option<&TxOut>>, FundingError> {
    if psbt.unsigned_tx.input.is_empty() {
        return Err(FundingError::Structure("no transaction inputs"));
    }
    if psbt.inputs.len() != psbt.unsigned_tx.input.len()
        || psbt.outputs.len() != psbt.unsigned_tx.output.len()
    {
        return Err(FundingError::Structure(
            "transaction and PSBT map counts differ",
        ));
    }
    if psbt
        .unsigned_tx
        .input
        .iter()
        .any(|input| !input.script_sig.is_empty() || !input.witness.is_empty())
    {
        return Err(FundingError::Structure(
            "unsigned transaction contains spending data",
        ));
    }
    let mut seen = BTreeSet::new();
    psbt.unsigned_tx
        .input
        .iter()
        .zip(&psbt.inputs)
        .enumerate()
        .map(|(index, (txin, input))| {
            if !seen.insert(txin.previous_output) {
                return Err(FundingError::DuplicateInput(index));
            }
            let previous = input
                .non_witness_utxo
                .as_ref()
                .map(|transaction| {
                    if transaction.compute_txid() != txin.previous_output.txid {
                        return Err(FundingError::PreviousTransaction(index));
                    }
                    transaction
                        .output
                        .get(txin.previous_output.vout as usize)
                        .ok_or(FundingError::PreviousTransaction(index))
                })
                .transpose()?;
            match (previous, input.witness_utxo.as_ref()) {
                (Some(left), Some(right)) if left != right => {
                    Err(FundingError::ConflictingOutputs(index))
                }
                (Some(output), _) | (_, Some(output)) => Ok(Some(output)),
                (None, None) => Ok(None),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{absolute, transaction, Amount, OutPoint, ScriptBuf, Transaction, TxIn};

    fn fixture() -> (Psbt, Transaction) {
        let previous = Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![TxIn::default()],
            output: vec![TxOut {
                value: Amount::from_sat(1_000),
                script_pubkey: ScriptBuf::new(),
            }],
        };
        let transaction = Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::new(previous.compute_txid(), 0),
                ..Default::default()
            }],
            output: previous.output.clone(),
        };
        (Psbt::from_unsigned_tx(transaction).unwrap(), previous)
    }

    #[test]
    fn missing_witness_and_full_previous_outputs_are_distinct() {
        let (mut psbt, previous) = fixture();
        assert_eq!(previous_outputs(&psbt).unwrap(), [None]);
        psbt.inputs[0].witness_utxo = Some(previous.output[0].clone());
        assert_eq!(
            previous_outputs(&psbt).unwrap(),
            [Some(&previous.output[0])]
        );
        psbt.inputs[0].non_witness_utxo = Some(previous.clone());
        assert_eq!(
            previous_outputs(&psbt).unwrap(),
            [Some(&previous.output[0])]
        );
        psbt.inputs[0].witness_utxo = None;
        assert_eq!(
            previous_outputs(&psbt).unwrap(),
            [Some(&previous.output[0])]
        );
    }

    #[test]
    fn full_previous_transactions_authenticate_identity_index_and_duplicate_evidence() {
        let (mut psbt, previous) = fixture();
        psbt.inputs[0].non_witness_utxo = Some(previous.clone());
        psbt.inputs[0].witness_utxo = Some(TxOut {
            value: Amount::from_sat(999),
            ..previous.output[0].clone()
        });
        assert_eq!(
            previous_outputs(&psbt),
            Err(FundingError::ConflictingOutputs(0))
        );
        psbt.inputs[0].witness_utxo = None;
        psbt.unsigned_tx.input[0].previous_output.vout = 1;
        assert_eq!(
            previous_outputs(&psbt),
            Err(FundingError::PreviousTransaction(0))
        );
        psbt.unsigned_tx.input[0].previous_output.vout = 0;
        psbt.inputs[0].non_witness_utxo.as_mut().unwrap().output[0].value = Amount::from_sat(998);
        assert_eq!(
            previous_outputs(&psbt),
            Err(FundingError::PreviousTransaction(0))
        );
    }
}
