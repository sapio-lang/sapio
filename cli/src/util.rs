// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use bitcoin::hashes::Hash;
use bitcoin::psbt::Psbt;
use serde::{de::DeserializeOwned, Serialize};
use std::error::Error;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub(crate) type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn input_name(path: Option<&Path>) -> String {
    path.filter(|path| *path != Path::new("-"))
        .map(|path| format!("'{}'", path.display()))
        .unwrap_or_else(|| "stdin".into())
}

pub(crate) fn read_input(path: Option<&Path>) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    match path {
        None => {
            std::io::stdin().read_to_end(&mut bytes)?;
        }
        Some(path) if path == Path::new("-") => {
            std::io::stdin().read_to_end(&mut bytes)?;
        }
        Some(path) => {
            bytes = std::fs::read(path)
                .map_err(|error| format!("cannot read '{}': {error}", path.display()))?
        }
    }
    Ok(bytes)
}

pub(crate) fn read_json<T: DeserializeOwned>(path: Option<&Path>) -> Result<T> {
    serde_json::from_slice(&read_input(path)?)
        .map_err(|error| format!("invalid JSON in {}: {error}", input_name(path)).into())
}

pub(crate) fn read_psbt(path: Option<&Path>) -> Result<Psbt> {
    let bytes = read_input(path)?;
    let encoded = std::str::from_utf8(&bytes)
        .map_err(|error| format!("invalid base64 text in {}: {error}", input_name(path)))?;
    let bytes = base64::decode(encoded.trim())
        .map_err(|error| format!("invalid base64 PSBT in {}: {error}", input_name(path)))?;
    Psbt::deserialize(&bytes)
        .map_err(|error| format!("invalid PSBT in {}: {error}", input_name(path)).into())
}

/// Create private files exclusively; argument-time existence checks cannot
/// prevent concurrent replacement of key material or collected signatures.
pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("cannot create '{}': {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("cannot write '{}': {error}", path.display()))?;
    Ok(())
}

pub(crate) fn write_output(path: Option<&Path>, bytes: &[u8]) -> Result<()> {
    match path {
        Some(path) if path != Path::new("-") => {
            let mut line = bytes.to_vec();
            line.push(b'\n');
            write_new(path, &line)?;
        }
        _ => {
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(bytes)?;
            stdout.write_all(b"\n")?;
        }
    }
    Ok(())
}

pub(crate) fn write_json(path: Option<&Path>, value: &impl Serialize) -> Result<()> {
    write_output(path, &serde_json::to_vec_pretty(value)?)
}

pub(crate) fn write_psbt(path: Option<&Path>, psbt: &Psbt) -> Result<()> {
    write_output(path, base64::encode(psbt.serialize()).as_bytes())
}

pub(crate) fn project_dirs() -> Result<directories::ProjectDirs> {
    directories::ProjectDirs::from("org", "judica", "sapio-cli")
        .ok_or_else(|| "cannot determine Sapio configuration and data directories".into())
}

pub(crate) fn module_path(workspace: Option<PathBuf>) -> Result<PathBuf> {
    let workspace = match workspace {
        Some(workspace) => workspace,
        None => project_dirs()?.data_dir().into(),
    };
    Ok(workspace.join("modules"))
}

/// Compute the key's CTV commitment from all known final scriptSigs.
/// Funding amounts and fees do not enter CTV, so key derivation does not
/// require the previous-output metadata needed by checked spend extraction.
pub fn ctv_hash(
    psbt: &Psbt,
) -> std::result::Result<bitcoin::hashes::sha256::Hash, sapio_psbt::PSBTValidationError> {
    use sapio_base::CTVHash;

    sapio_psbt::validate_psbt(psbt)?;
    let mut transaction = psbt.unsigned_tx.clone();
    for (input, metadata) in transaction.input.iter_mut().zip(&psbt.inputs) {
        if let Some(script_sig) = &metadata.final_script_sig {
            input.script_sig = script_sig.clone();
        }
    }
    Ok(transaction.get_ctv_hash(0))
}

pub(crate) fn create_mock_output() -> bitcoin::OutPoint {
    bitcoin::OutPoint {
        txid: bitcoin::hashes::sha256d::Hash::from_byte_array(
            bitcoin::hashes::sha256::Hash::hash(format!("mock:{}", 0).as_bytes()).to_byte_array(),
        )
        .into(),
        vout: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{absolute, transaction, Transaction, TxIn};
    use sapio_base::CTVHash;

    fn psbt() -> Psbt {
        Psbt::from_unsigned_tx(Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![TxIn::default(), TxIn::default()],
            output: vec![],
        })
        .unwrap()
    }

    #[test]
    fn covenant_key_uses_every_final_scriptsig_without_requiring_fee_metadata() {
        let mut psbt = psbt();
        assert!(psbt.clone().extract_tx().is_err());
        let unsigned_hash = ctv_hash(&psbt).unwrap();
        let script = bitcoin::script::Builder::new().push_int(1).into_script();
        psbt.inputs[1].final_script_sig = Some(script.clone());
        let mut expected = psbt.unsigned_tx.clone();
        expected.input[1].script_sig = script;
        assert_ne!(ctv_hash(&psbt).unwrap(), unsigned_hash);
        assert_eq!(ctv_hash(&psbt).unwrap(), expected.get_ctv_hash(0));
        assert!(psbt.unsigned_tx.input[1].script_sig.is_empty());
        psbt.inputs.pop();
        assert!(ctv_hash(&psbt).is_err());
    }

    #[test]
    fn output_creation_never_replaces_an_existing_file() {
        let path = std::env::temp_dir().join(format!(
            "sapio-output-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        write_new(&path, b"original").unwrap();
        assert!(write_output(Some(&path), b"replacement").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::remove_file(path).unwrap();
    }
}
