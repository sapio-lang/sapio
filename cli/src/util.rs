// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use bitcoin::hashes::Hash;
use bitcoin::psbt::Psbt;
use std::error::Error;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Exclusively create a secret file, with owner-only permissions on Unix.
///
/// Existing files (including symlinks) are never followed or overwritten. Set
/// permissions through the open handle, so a pathname replacement cannot cause
/// us to change the permissions of another file.
pub async fn write_new_secret(path: impl AsRef<Path>, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .await?;
    }
    file.write_all(bytes).await?;
    file.sync_all().await
}

/// Read secret bytes and warn if the opened file is accessible to other Unix users.
pub async fn read_secret(path: impl AsRef<Path>) -> std::io::Result<Vec<u8>> {
    let path = path.as_ref();
    let mut file = tokio::fs::File::open(path).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if file.metadata().await?.permissions().mode() & 0o077 != 0 {
            eprintln!(
                "Warning: secret file {} is accessible to other users; restrict its permissions to 0600.",
                path.display()
            );
        }
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).await?;
    Ok(bytes)
}
/// Checks that a file exists during argument parsing
///
/// **Race Conditions** if file is deleted after this call
pub fn check_file(p: &str) -> Result<(), String> {
    std::fs::metadata(p).map_err(|_| String::from("File doesn't exist"))?;
    Ok(())
}
/// Checks that a file does not exist during argument parsing
///
/// **Race Conditions** if file is created after this call
pub fn check_file_not(p: &str) -> Result<(), String> {
    if std::fs::metadata(p).is_ok() {
        return Err(String::from("File exists already"));
    }
    Ok(())
}

/// Reads a PSBT from a file and checks that it is correctly formatted
pub fn decode_psbt_file(a: &clap::ArgMatches, b: &str) -> Result<Psbt, Box<dyn std::error::Error>> {
    let bytes = std::fs::read_to_string(a.value_of_os(b).unwrap())?;
    let bytes = base64::decode(bytes.trim())?;
    let psbt = Psbt::deserialize(&bytes)?;
    Ok(psbt)
}

/// Reads a PSBT either from a string or from stdin
pub async fn get_psbt_from(psbt_str: Option<&str>) -> Result<Psbt, Box<dyn Error>> {
    let encoded = if let Some(psbt) = psbt_str {
        psbt.to_owned()
    } else {
        let mut encoded = String::new();
        tokio::io::stdin().read_to_string(&mut encoded).await?;
        encoded
    };
    let psbt = Psbt::deserialize(&base64::decode(encoded.trim())?)?;
    Ok(psbt)
}

/// Compute the key's CTV commitment from all known final scriptSigs.
/// Funding amounts and fees do not enter CTV, so key derivation does not
/// require the previous-output metadata needed by checked spend extraction.
pub fn ctv_hash(
    psbt: &Psbt,
) -> Result<bitcoin::hashes::sha256::Hash, sapio_psbt::PSBTValidationError> {
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

/// get the path for the compiled modules
pub(crate) fn get_data_dir(typ: &str, org: &str, proj: &str) -> PathBuf {
    let proj =
        directories::ProjectDirs::from(typ, org, proj).expect("Failed to find config directory");
    proj.data_dir().into()
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

    #[cfg(unix)]
    #[tokio::test]
    async fn secret_writes_are_private_exclusive_and_do_not_follow_symlinks() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let directory = std::env::temp_dir().join(format!(
            "sapio-secret-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("config.json");
        let secret = br#"{"rpc_password":"private"}"#;
        write_new_secret(&path, secret).await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), secret);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            write_new_secret(&path, b"replacement")
                .await
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(&path).unwrap(), secret);

        let link = directory.join("link");
        symlink(&path, &link).unwrap();
        assert_eq!(
            write_new_secret(&link, b"replacement")
                .await
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(&path).unwrap(), secret);
        let missing = directory.join("missing");
        let dangling = directory.join("dangling");
        symlink(&missing, &dangling).unwrap();
        assert_eq!(
            write_new_secret(&dangling, b"secret")
                .await
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert!(!missing.exists());

        let concurrent = directory.join("concurrent");
        let (first, second) = tokio::join!(
            write_new_secret(&concurrent, b"first"),
            write_new_secret(&concurrent, b"second")
        );
        assert_ne!(first.is_ok(), second.is_ok());
        let (winner, loser) = if first.is_ok() {
            (b"first".as_slice(), second)
        } else {
            (b"second".as_slice(), first)
        };
        assert_eq!(loser.unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&concurrent).unwrap(), winner);
        std::fs::remove_dir_all(directory).unwrap();
    }

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

    #[tokio::test]
    async fn psbt_text_input_consumes_the_complete_binary_encoding() {
        let psbt = psbt();
        let encoded = format!("{}\n", base64::encode(psbt.serialize()));
        assert_eq!(get_psbt_from(Some(&encoded)).await.unwrap(), psbt);
        let mut trailing = psbt.serialize();
        trailing.push(0);
        assert!(get_psbt_from(Some(&base64::encode(trailing)))
            .await
            .is_err());
    }
}
