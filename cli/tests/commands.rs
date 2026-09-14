//! Process-level command and first-run contracts, without a wallet or node.

use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "sapio CLI commands {} {}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sapio-cli"));
        command.current_dir(&self.0).stdin(Stdio::null());
        command
    }

    fn input(&self, args: &[&str], input: &str) -> Output {
        let mut child = self
            .command()
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn local_modules_ignore_runtime_config_and_report_real_operation_failures() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .args([
            "--config",
            "does not exist.json",
            "contract",
            "list",
            "--workspace",
            ".",
        ])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        serde_json::json!({})
    );
    assert!(!fixture.0.join("does not exist.json").exists());

    let output = fixture
        .command()
        .args([
            "--config",
            "does not exist.json",
            "contract",
            "load",
            "--workspace",
            ".",
            "--file",
            "missing module.wasm",
            "--output",
            "artifact.json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
    assert!(!fixture.0.join("artifact.json").exists());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("configuration"));
}

#[test]
fn create_checks_public_network_before_opening_the_module() {
    let fixture = Fixture::new();
    let output = fixture.input(
        &[
            "contract",
            "create",
            "--workspace",
            ".",
            "--file",
            "missing.wasm",
            "--network",
            "bitcoin",
        ],
        include_str!("../../contrib/vectors/clause_input.json"),
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("context.network"));
}

#[test]
fn new_keys_are_private_and_never_replaced() {
    let fixture = Fixture::new();
    let args = [
        "signer",
        "new",
        "--network",
        "regtest",
        "--output",
        "signer.key",
    ];
    let output = fixture.command().args(args).output().unwrap();
    success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("tpub"));
    let path = fixture.0.join("signer.key");
    let original = std::fs::read(&path).unwrap();
    let output = fixture.command().args(args).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn wizard_honors_custom_file_and_show_redacts_passwords() {
    let fixture = Fixture::new();
    let args = ["--config", "custom.json", "configure", "wizard", "--write"];
    let input = "regtest\nhttp://127.0.0.1:18443\n/local/bitcoin.cookie\nnative_ctv_research\n";
    let output = fixture.input(&args, input);
    success(&output);
    assert!(output.stdout.is_empty());
    let original = std::fs::read(fixture.0.join("custom.json")).unwrap();
    let config: Value = serde_json::from_slice(&original).unwrap();
    assert_eq!(
        config["regtest"]["api_node"]["auth"]["CookieFile"],
        "/local/bitcoin.cookie"
    );
    let output = fixture.input(&args, input);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        std::fs::read(fixture.0.join("custom.json")).unwrap(),
        original
    );

    std::fs::write(
        fixture.0.join("basic.json"),
        include_str!("../../contrib/vectors/basic_config.json"),
    )
    .unwrap();
    let output = fixture
        .command()
        .args(["--config", "basic.json", "configure", "show"])
        .output()
        .unwrap();
    success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("<redacted>"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("examplepassword"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("examplepassword"));
}

#[tokio::test]
async fn wizard_eof_aborts_without_hanging_or_writing() {
    let fixture = Fixture::new();
    for input in [
        "",
        "regtest\n",
        "regtest\nhttp://localhost:18443\n",
        "regtest\nhttp://localhost:18443\n/cookie\n",
    ] {
        use tokio::io::AsyncWriteExt;
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_sapio-cli"))
            .current_dir(&fixture.0)
            .args([
                "--config",
                "must not exist.json",
                "configure",
                "wizard",
                "--write",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .await
            .unwrap();
        let output =
            tokio::time::timeout(std::time::Duration::from_secs(5), child.wait_with_output())
                .await
                .unwrap()
                .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(!fixture.0.join("must not exist.json").exists());
    }
}

#[test]
fn studio_retains_response_envelopes_and_cli_uses_direct_payloads() {
    let fixture = Fixture::new();
    let request = serde_json::json!({
        "context": {"path": fixture.0.join("modules"), "module_locator": null, "net": "regtest", "plugin_map": null},
        "command": {"List": null},
    });
    let output = fixture.input(&["studio", "server", "--stdin"], &request.to_string());
    success(&output);
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        serde_json::json!({"result": {"Ok": {"List": {"items": {}}}}})
    );

    let files = fixture
        .command()
        .args(["configure", "files", "--json"])
        .output()
        .unwrap();
    success(&files);
    let files: Value = serde_json::from_slice(&files.stdout).unwrap();
    let dirs = directories::ProjectDirs::from("org", "judica", "sapio-cli").unwrap();
    assert_eq!(
        files["modules"],
        dirs.data_dir().join("modules").to_str().unwrap()
    );
}
