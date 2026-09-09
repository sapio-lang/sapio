use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::time::timeout;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "sapio-emulator-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir(&path).unwrap();
        let mut config: Value =
            serde_json::from_str(include_str!("../../contrib/vectors/basic_config.json")).unwrap();
        config["regtest"]["emulator_nodes"]["enabled"] = true.into();
        config["regtest"]["emulator_nodes"]["emulators"][0][1] = "address-without-a-port".into();
        std::fs::write(
            path.join("config.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        std::fs::write(path.join("seed"), [44; 32]).unwrap();
        Self(path)
    }

    fn server(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sapio-cli"));
        command
            .arg("--config")
            .arg(self.0.join("config.json"))
            .args(["emulator", "server"])
            .arg(self.0.join("seed"))
            .arg("127.0.0.1:0")
            .kill_on_drop(true);
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[tokio::test]
async fn server_binds_without_resolving_unused_peers_and_applies_limits() {
    let fixture = Fixture::new();
    let mut child = fixture
        .server()
        .args(["--request-timeout-secs", "1", "--max-connections", "1"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    timeout(Duration::from_secs(10), output.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let status: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(status["request_timeout_secs"], 1);
    assert_eq!(status["max_connections"], 1);
    let address: std::net::SocketAddr = status["interface"].as_str().unwrap().parse().unwrap();
    assert_ne!(address.port(), 0);
    assert!(status["pk"].as_str().unwrap().starts_with("tpub"));

    // An idle connection expires and returns its only slot for another peer.
    for _ in 0..2 {
        let mut stream = TcpStream::connect(address).await.unwrap();
        assert_eq!(
            timeout(Duration::from_secs(10), stream.read(&mut [0]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert!(child.try_wait().unwrap().is_none());
    }
    child.kill().await.unwrap();
    child.wait().await.unwrap();
}

#[tokio::test]
async fn invalid_limits_fail_without_announcing_readiness() {
    let fixture = Fixture::new();
    for flag in ["--request-timeout-secs", "--max-connections"] {
        let output = timeout(
            Duration::from_secs(10),
            fixture.server().args([flag, "0"]).output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("InvalidInput"));
    }
}
