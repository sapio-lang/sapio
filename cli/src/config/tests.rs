use super::*;

fn emulator_config() -> EmulatorConfig {
    serde_json::from_value(
        serde_json::from_str::<serde_json::Value>(include_str!(
            "../../../contrib/vectors/basic_config.json"
        ))
        .unwrap()["regtest"]["emulator_nodes"]
            .clone(),
    )
    .unwrap()
}

#[tokio::test]
async fn numeric_peers_resolve_without_opening_a_connection() {
    let mut config = emulator_config();
    assert_eq!(config.request_timeout_secs, 30);
    config.emulators[0].1 = "127.0.0.1:0".into();
    let emulator = config.get_emulator().await.unwrap();
    let hash = bitcoin::hashes::Hash::from_slice(&[7; 32]).unwrap();
    assert!(emulator.get_signer_for(hash).is_ok());
    let root = ExtendedPubKey::from_priv(
        &bitcoin::secp256k1::Secp256k1::new(),
        &bitcoin::util::bip32::ExtendedPrivKey::new_master(bitcoin::Network::Regtest, &[8; 32])
            .unwrap(),
    );
    config.emulators.push((root, "127.0.0.1:0".into()));
    config.threshold = 2;
    assert!(config
        .get_emulator()
        .await
        .unwrap()
        .get_signer_for(hash)
        .is_ok());
}

#[tokio::test]
async fn invalid_thresholds_and_timeouts_fail_before_resolving_peers() {
    let original = emulator_config();
    for threshold in [0, 2] {
        let mut config = original.clone();
        config.threshold = threshold;
        let error = config.get_emulator().await.err().unwrap().to_string();
        assert!(error.contains("threshold"), "{error}");
    }
    let mut empty = original.clone();
    empty.emulators.clear();
    assert!(empty
        .get_emulator()
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("threshold"));
    for request_timeout_secs in [0, u64::MAX] {
        let mut config = original.clone();
        config.request_timeout_secs = request_timeout_secs;
        let error = config.get_emulator().await.err().unwrap().to_string();
        assert!(error.contains("timeout"), "{error}");
    }
}

#[tokio::test]
async fn malformed_peer_addresses_return_errors() {
    let mut config = emulator_config();
    config.emulators[0].1 = "address-without-a-port".into();
    assert!(config.get_emulator().await.is_err());
}
