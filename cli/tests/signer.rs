use bitcoin::bip32::Xpriv;
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{Amount, Network, ScriptBuf, Transaction, TxIn, TxOut};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn signer_reads_a_psbt_file_or_stdin_and_produces_a_valid_signature() {
    let directory = std::env::temp_dir().join(format!(
        "sapio-signer-test-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir(&directory).unwrap();
    let fixture = Fixture(directory);
    let secp = Secp256k1::new();
    let root = Xpriv::new_master(Network::Regtest, &[42; 32]).unwrap();
    let key = root.to_keypair(&secp).x_only_public_key().0;
    let prevout = TxOut {
        value: Amount::from_sat(2_000),
        script_pubkey: ScriptBuf::new_p2tr(&secp, key, None),
    };
    let transaction = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: vec![TxIn::default()],
        output: vec![TxOut {
            value: Amount::from_sat(1_900),
            script_pubkey: prevout.script_pubkey.clone(),
        }],
    };
    let mut psbt = Psbt::from_unsigned_tx(transaction.clone()).unwrap();
    psbt.inputs[0].witness_utxo = Some(prevout);
    psbt.inputs[0].tap_internal_key = Some(key);
    let encoded = format!("{}\n", base64::encode(psbt.serialize()));
    let key_path = fixture.0.join("key");
    let psbt_path = fixture.0.join("unsigned.psbt");
    std::fs::write(&key_path, root.encode()).unwrap();
    std::fs::write(&psbt_path, &encoded).unwrap();

    for from_file in [true, false] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sapio-cli"));
        command.args(["signer", "sign", "--key"]).arg(&key_path);
        let output_path = fixture.0.join("signed.psbt");
        if from_file {
            command
                .arg("--psbt")
                .arg(&psbt_path)
                .arg("--output")
                .arg(&output_path);
        } else {
            command.stdin(Stdio::piped());
        }
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if !from_file {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(encoded.as_bytes())
                .unwrap();
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let signed = if from_file {
            assert!(output.stdout.is_empty());
            std::fs::read_to_string(&output_path).unwrap()
        } else {
            String::from_utf8(output.stdout).unwrap()
        };
        let signed = Psbt::deserialize(&base64::decode(signed.trim()).unwrap()).unwrap();
        assert_eq!(signed.unsigned_tx, transaction);
        assert!(signed.inputs[0].tap_key_sig.is_some());
        let finalized = sapio_psbt::finalize::finalize(signed, &secp).unwrap();
        let extracted = finalized.extract_tx().unwrap();
        assert_eq!(extracted.input[0].witness.len(), 1);
        assert_eq!(extracted.output, transaction.output);
    }
}
