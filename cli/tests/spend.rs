use bitcoin::bip32::{Xpriv, Xpub};
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{Amount, Network, ScriptBuf, Transaction, TxIn, TxOut, XOnlyPublicKey};
use emulator_connect::program::{
    ProgramCapability, ProgramEvidence, ProgramOracle, ProgramSigningRequest, ProgramSpendPath,
    SpendAssets,
};
use sapio::contract::{Compilable, CompilationError, Context};
use sapio_base::fragments::{template_hash, templatehash_wasm_instance};
use sapio_base::policy::ScriptPolicy;
use sapio_base::program::EmulatedProgram;
use sapio_base::{Clause, LoweringPlan};
use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::Arc;

struct Source {
    policy: ScriptPolicy,
    native: XOnlyPublicKey,
}

#[sapio::contract]
impl Source {
    #[spend]
    fn spend(&self) -> ScriptPolicy {
        self.policy.clone()
    }

    #[spend]
    fn cooperative(&self) -> Clause {
        Clause::Key(self.native)
    }

    #[internal_key]
    fn internal_key(&self, _ctx: &Context) -> Result<Option<XOnlyPublicKey>, CompilationError> {
        Ok(Some(self.native))
    }
}

struct Fixture {
    directory: PathBuf,
    roots: Vec<Xpriv>,
    native: XOnlyPublicKey,
    path: String,
}

impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "sapio-completion-cli-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(
            directory.join("invalid-config.json"),
            "invalid configuration",
        )
        .unwrap();
        let secp = Secp256k1::new();
        let native_root = Xpriv::new_master(Network::Regtest, &[29; 32]).unwrap();
        let native = native_root.to_keypair(&secp).x_only_public_key().0;
        let roots: Vec<_> = [30, 31]
            .into_iter()
            .map(|byte| Xpriv::new_master(Network::Regtest, &[byte; 32]).unwrap())
            .collect();
        let transaction = Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![TxIn::default()],
            output: vec![TxOut {
                value: Amount::from_sat(900),
                script_pubkey: ScriptBuf::new(),
            }],
        };
        let mut policies: Vec<ScriptPolicy> = vec![Clause::Key(native).into()];
        let instance = templatehash_wasm_instance(
            template_hash(&transaction, 0, Some(&[0x50, 7, 8])).unwrap(),
        );
        policies.extend(roots.iter().map(|root| {
            ScriptPolicy::from(
                EmulatedProgram::new(instance.clone(), Xpub::from_priv(&secp, root)).unwrap(),
            )
        }));
        let artifact = Source {
            policy: ScriptPolicy::And(policies),
            native,
        }
        .compile(Context::new(
            Network::Regtest,
            Amount::from_sat(1_000),
            LoweringPlan::Native,
            "cli_completion".try_into().unwrap(),
            Arc::new(Default::default()),
            None,
        ))
        .unwrap();
        let requirements = artifact.program_requirements().unwrap();
        assert_eq!(requirements.len(), 2);
        let ProgramSpendPath::ScriptPath(leaf) = requirements.iter().next().unwrap().path else {
            panic!("script expected")
        };
        let mut psbt = Psbt::from_unsigned_tx(transaction).unwrap();
        psbt.inputs[0].witness_utxo = Some(TxOut {
            value: Amount::from_sat(1_000),
            script_pubkey: (&artifact.address).into(),
        });
        sapio_psbt::annex::set(&mut psbt.inputs[0], Some(vec![0x50, 7, 8])).unwrap();
        let assets = SpendAssets {
            schnorr_keys: [native].into(),
            programs: requirements
                .iter()
                .map(|requirement| ProgramCapability {
                    requirement: requirement.clone(),
                    codec: "templatehash/empty-v1".into(),
                    evidence_available: true,
                    signer_available: true,
                })
                .collect(),
            ..Default::default()
        };
        let evidence: Vec<_> = requirements
            .into_iter()
            .map(|requirement| ProgramEvidence {
                requirement,
                codec: "templatehash/empty-v1".into(),
                witness: vec![],
            })
            .collect();
        for (name, bytes) in [
            ("artifact.json", serde_json::to_vec(&artifact).unwrap()),
            ("funded.psbt", base64::encode(psbt.serialize()).into_bytes()),
            ("assets.json", serde_json::to_vec(&assets).unwrap()),
            ("evidence.json", serde_json::to_vec(&evidence).unwrap()),
            ("native.key", native_root.encode().to_vec()),
        ] {
            std::fs::write(directory.join(name), bytes).unwrap();
        }
        Self {
            directory,
            roots,
            native,
            path: format!("script:{leaf}"),
        }
    }

    fn command(&self, operation: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sapio-cli"));
        command.current_dir(&self.directory).args([
            "--config",
            "invalid-config.json",
            "contract",
            "spend",
            operation,
            "--artifact",
            "artifact.json",
        ]);
        if operation != "prepare" {
            command.args(["--intent", "intent.json"]);
        }
        command
    }

    fn run(&self, operation: &str, args: &[&str]) -> Output {
        let result = self.command(operation).args(args).output().unwrap();
        assert!(
            result.status.success(),
            "{operation}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        result
    }

    fn prepare(&self) {
        self.run(
            "prepare",
            &[
                "--psbt",
                "funded.psbt",
                "--assets",
                "assets.json",
                "--evidence",
                "evidence.json",
                "--path",
                &self.path,
                "--output",
                "intent.json",
                "--psbt-output",
                "baseline.psbt",
            ],
        );
    }

    fn psbt(&self, name: &str) -> Psbt {
        Psbt::deserialize(
            &base64::decode(
                std::fs::read_to_string(self.directory.join(name))
                    .unwrap()
                    .trim(),
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn responses(&self) {
        let list: Value = serde_json::from_slice(&self.run("requests", &[]).stdout).unwrap();
        assert_eq!(list.as_array().unwrap().len(), 2);
        for index in 0..2 {
            let bytes = self
                .run("requests", &["--index", &index.to_string()])
                .stdout;
            let request: ProgramSigningRequest = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(list[index]["index"], index);
            let requirement: sapio::contract::abi::object::ProgramRequirement =
                serde_json::from_value(list[index]["requirement"].clone()).unwrap();
            assert_eq!(*requirement.program.instance(), request.instance);
            let root = self
                .roots
                .iter()
                .find(|root| {
                    Xpub::from_priv(&Secp256k1::new(), root) == *requirement.program.root()
                })
                .expect("request names an explicitly configured signer");
            assert_eq!(requirement.path, request.path);
            assert_eq!(
                list[index]["request"],
                serde_json::to_value(&request).unwrap()
            );
            let response = ProgramOracle::new(*root, vec![])
                .unwrap()
                .sign(request)
                .unwrap();
            std::fs::write(
                self.directory.join(format!("response{index}.psbt")),
                base64::encode(response.serialize()),
            )
            .unwrap();
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}

#[test]
fn independent_processes_resume_merge_out_of_order_and_sign_only_selected_slots() {
    let fixture = Fixture::new();
    fixture.prepare();
    fixture.responses();
    let pending: Value = serde_json::from_slice(&fixture.run("status", &[]).stdout).unwrap();
    assert_eq!(pending["status"], "MissingAssets");
    fixture.run(
        "apply",
        &["--response", "1=response1.psbt", "--output", "first.psbt"],
    );
    fixture.run(
        "apply",
        &[
            "--psbt",
            "first.psbt",
            "--response",
            "0=response0.psbt",
            "--output",
            "both.psbt",
        ],
    );
    assert_eq!(fixture.psbt("both.psbt").inputs[0].tap_script_sigs.len(), 2);
    fixture.run(
        "sign-native",
        &[
            "--psbt",
            "both.psbt",
            "--key",
            "native.key",
            "--output",
            "signed.psbt",
        ],
    );
    let signed = fixture.psbt("signed.psbt");
    assert_eq!(signed.inputs[0].tap_script_sigs.len(), 3);
    assert!(signed.inputs[0]
        .tap_script_sigs
        .keys()
        .any(|(key, _)| *key == fixture.native));
    assert!(
        signed.inputs[0].tap_key_sig.is_none(),
        "same available internal key must remain unsigned"
    );
    let ready: Value =
        serde_json::from_slice(&fixture.run("status", &["--psbt", "signed.psbt"]).stdout).unwrap();
    assert_eq!(ready["status"], "Planned");
    fixture.run(
        "finalize",
        &["--psbt", "signed.psbt", "--output", "final.psbt"],
    );
    let transaction = fixture.psbt("final.psbt").extract_tx().unwrap();
    assert_eq!(transaction.input[0].witness.len(), 6);
    assert_eq!(transaction.input[0].witness.last(), Some(&[0x50, 7, 8][..]));
    let hex = fixture.run("finalize", &["--psbt", "signed.psbt", "--transaction"]);
    assert_eq!(
        String::from_utf8(hex.stdout).unwrap().trim(),
        bitcoin::consensus::encode::serialize_hex(&transaction)
    );
}

#[test]
fn rejected_batch_or_incomplete_completion_never_writes_an_output() {
    let fixture = Fixture::new();
    fixture.prepare();
    fixture.responses();
    let baseline = fixture.psbt("baseline.psbt");
    let rejected = fixture
        .command("apply")
        .args([
            "--response",
            "0=response0.psbt",
            "--response",
            "99=response1.psbt",
            "--output",
            "rejected.psbt",
        ])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(!fixture.directory.join("rejected.psbt").exists());
    assert_eq!(fixture.psbt("baseline.psbt"), baseline);
    let incomplete = fixture
        .command("finalize")
        .args(["--output", "incomplete.psbt"])
        .output()
        .unwrap();
    assert!(!incomplete.status.success());
    assert!(!fixture.directory.join("incomplete.psbt").exists());
    std::fs::write(fixture.directory.join("existing.psbt"), b"keep me").unwrap();
    let overwrite = fixture
        .command("apply")
        .args([
            "--response",
            "0=response0.psbt",
            "--output",
            "existing.psbt",
        ])
        .output()
        .unwrap();
    assert!(!overwrite.status.success());
    assert_eq!(
        std::fs::read(fixture.directory.join("existing.psbt")).unwrap(),
        b"keep me"
    );
}
