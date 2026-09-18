#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn new_keys_are_private_even_with_a_permissive_umask_and_old_keys_warn() {
    let directory = std::env::temp_dir().join(format!(
        "sapio-key-permissions-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir(&directory).unwrap();
    let fixture = Fixture(directory);
    let key_path = fixture.0.join("key");
    // Change only the child process's umask; tests may run in parallel.
    let output = Command::new("sh")
        .args([
            "-c",
            "umask 000; exec \"$1\" signer new --network regtest --output \"$2\"",
            "sapio-key-test",
        ])
        .arg(env!("CARGO_BIN_EXE_sapio-cli"))
        .arg(&key_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::metadata(&key_path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let original = std::fs::read(&key_path).unwrap();
    let duplicate = Command::new(env!("CARGO_BIN_EXE_sapio-cli"))
        .args(["signer", "new", "--network", "regtest", "--output"])
        .arg(&key_path)
        .output()
        .unwrap();
    assert!(!duplicate.status.success());
    assert_eq!(std::fs::read(&key_path).unwrap(), original);

    for mode in [0o600, 0o640, 0o604] {
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(mode)).unwrap();
        let shown = Command::new(env!("CARGO_BIN_EXE_sapio-cli"))
            .args(["signer", "show", "--input"])
            .arg(&key_path)
            .output()
            .unwrap();
        assert!(shown.status.success());
        assert_eq!(shown.stdout, output.stdout);
        assert_eq!(
            String::from_utf8_lossy(&shown.stderr).contains("restrict its permissions to 0600"),
            mode != 0o600
        );
    }
}
