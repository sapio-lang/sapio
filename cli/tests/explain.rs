use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
use bitcoin::{Address, Amount, Network, TxOut};
use sapio::contract::{Compilable, CompilationError, Compiled, Context};
use sapio::template::{OutputAmount, Template};
use sapio_base::covenant::LoweringPlan;
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "sapio-explain-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("invalid-config.json"), "not a configuration").unwrap();
        Self(path)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sapio-cli"));
        command
            .arg("--config")
            .arg(self.0.join("invalid-config.json"))
            .args(["contract", "explain"]);
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

struct Payment;

#[sapio::contract]
impl Payment {
    #[action(committed)]
    fn pay(&self, ctx: Context) -> Result<Template, CompilationError> {
        let key = Keypair::from_secret_key(
            &Secp256k1::new(),
            &SecretKey::from_slice(&[17; 32]).unwrap(),
        )
        .x_only_public_key()
        .0;
        let destination = Compiled::from_address(
            Address::p2tr(&Secp256k1::new(), key, None, Network::Regtest),
            Amount::ZERO,
        );
        let mut plan = ctx.template_plan();
        plan.output(
            "first",
            OutputAmount::Exact(Amount::from_sat(400)),
            &destination,
        )?;
        plan.output("second", OutputAmount::Remainder, &destination)?;
        plan.reserve_fees(Amount::from_sat(100));
        Ok(plan.finish()?)
    }
}

fn artifact() -> Compiled {
    Payment
        .compile(Context::new(
            Network::Regtest,
            Amount::from_sat(1_000),
            LoweringPlan::Native,
            "explain".try_into().unwrap(),
            Arc::new(Default::default()),
            None,
        ))
        .unwrap()
}

#[test]
fn explains_file_or_stdin_without_loading_configuration() {
    let fixture = Fixture::new();
    let mut artifact = artifact();
    let expected = artifact.explain().unwrap();
    artifact
        .metadata
        .extra
        .insert("claimed_authorization".into(), serde_json::json!("ignored"));
    assert_eq!(artifact.explain().unwrap(), expected);
    let bytes = serde_json::to_vec(&artifact).unwrap();
    let artifact_path = fixture.0.join("artifact.json");
    std::fs::write(&artifact_path, &bytes).unwrap();
    for from_stdin in [false, true] {
        let mut command = fixture.command();
        command
            .arg("--json")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if from_stdin {
            command.stdin(Stdio::piped());
        } else {
            command.arg("--file").arg(&artifact_path);
        }
        let mut child = command.spawn().unwrap();
        if from_stdin {
            child.stdin.take().unwrap().write_all(&bytes).unwrap();
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(report.get("spend").is_none());
        assert_eq!(
            report["artifact"]["nodes"][0]["templates"][0]["outputs"][0]["name"],
            "first"
        );
        assert_eq!(
            report["artifact"]["nodes"][0]["templates"][0]["outputs"][1]["amount_sats"],
            500
        );
        assert_eq!(
            report["artifact"]["nodes"][0]["actions"][0]["kind"],
            "committed"
        );
        assert!(report["artifact"]["nodes"][0]["actions"][0]["schema"].is_object());
        let original: Value = serde_json::from_slice(&bytes).unwrap();
        let nodes = report["artifact"]["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 3);
        assert_ne!(nodes[1]["location"], nodes[2]["location"]);
        assert_eq!(nodes[1]["source_path"], nodes[2]["source_path"]);
        for node in nodes {
            assert!(original
                .pointer(node["location"].as_str().unwrap())
                .is_some());
        }
    }
    let output = fixture
        .command()
        .arg("--file")
        .arg(&artifact_path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Local fee cap: 100 sat"));
    assert!(text.contains("Output 1 (second): 500 sat"));

    artifact.ctv_to_tx.values_mut().next().unwrap().tx.output[0].value += Amount::ONE_SAT;
    std::fs::write(&artifact_path, serde_json::to_vec(&artifact).unwrap()).unwrap();
    let rejected = fixture
        .command()
        .arg("--file")
        .arg(&artifact_path)
        .arg("--json")
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
}

#[test]
fn funded_explanation_reports_actual_fees_and_rejects_the_fee_cap() {
    let fixture = Fixture::new();
    let artifact = artifact();
    let artifact_path = fixture.0.join("artifact.json");
    let psbt_path = fixture.0.join("funded.psbt");
    let assets_path = fixture.0.join("assets.json");
    std::fs::write(&artifact_path, serde_json::to_vec(&artifact).unwrap()).unwrap();
    std::fs::write(&assets_path, "{}").unwrap();
    let mut psbt =
        Psbt::from_unsigned_tx(artifact.ctv_to_tx.values().next().unwrap().tx.clone()).unwrap();
    psbt.inputs[0].witness_utxo = Some(TxOut {
        value: Amount::from_sat(1_000),
        script_pubkey: (&artifact.address).into(),
    });
    std::fs::write(&psbt_path, base64::encode(psbt.serialize())).unwrap();
    let output = fixture
        .command()
        .arg("--file")
        .arg(&artifact_path)
        .arg("--psbt")
        .arg(&psbt_path)
        .arg("--assets")
        .arg(&assets_path)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["spend"]["funding"], "Met");
    assert_eq!(report["spend"]["template_funding"]["actual_fee_sats"], 100);
    assert_eq!(report["spend"]["template_funding"]["maximum_fee_sats"], 100);
    assert!(!report["spend"]["branches"].as_array().unwrap().is_empty());

    psbt.inputs[0].witness_utxo.as_mut().unwrap().value += Amount::ONE_SAT;
    std::fs::write(&psbt_path, base64::encode(psbt.serialize())).unwrap();
    let rejected = fixture
        .command()
        .arg("--file")
        .arg(&artifact_path)
        .arg("--psbt")
        .arg(&psbt_path)
        .arg("--json")
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr)
        .to_lowercase()
        .contains("fee"));
}
