use super::*;

fn emulator_config() -> EmulatorConfig {
    serde_json::from_value(serde_json::json!({
        "emulators": [[
            "tpubD6NzVbkrYhZ4Wf398td3H8YhWBsXx9Sxa4W3cQWkNW3N3DHSNB2qtPoUMXrA6JNaPxodQfRpoZNE5tGM9iZ4xfUEFRJEJvfs8W5paUagYCE",
            "example.please.change.this.before.using:8367"
        ]],
        "threshold": 1
    }))
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
    let root = Xpub::from_priv(
        &bitcoin::secp256k1::Secp256k1::new(),
        &bitcoin::bip32::Xpriv::new_master(bitcoin::Network::Regtest, &[8; 32]).unwrap(),
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

#[tokio::test]
async fn invalid_runtime_signer_roots_fail_before_resolving_peers() {
    use sapio_base::covenant::CovenantError;

    let mut deep = emulator_config();
    deep.emulators[0].0.depth = 247;
    deep.emulators[0].1 = "address-without-a-port".into();
    let error = deep.get_emulator().await.err().unwrap();
    assert!(
        matches!(
            error.downcast_ref::<CovenantError>(),
            Some(CovenantError::RootDepth {
                index: 0,
                depth: 247
            })
        ),
        "{error}"
    );

    let mut duplicate = emulator_config();
    duplicate.emulators[0].1 = "address-without-a-port".into();
    duplicate.emulators.push(duplicate.emulators[0].clone());
    duplicate.threshold = 2;
    let error = duplicate.get_emulator().await.err().unwrap();
    assert!(
        matches!(
            error.downcast_ref::<CovenantError>(),
            Some(CovenantError::DuplicateSigner {
                first: 0,
                second: 1
            })
        ),
        "{error}"
    );
}

#[tokio::test]
async fn explicit_backend_selection_controls_the_lowering_policy() {
    let hash = bitcoin::hashes::Hash::from_slice(&[7; 32]).unwrap();
    let native: CovenantConfig =
        serde_json::from_value(serde_json::json!({"mode": "native_ctv_research"})).unwrap();
    assert_eq!(
        native
            .get_emulator()
            .await
            .unwrap()
            .get_signer_for(hash)
            .unwrap(),
        sapio_base::Clause::TxTemplate(hash)
    );

    let mut signer = emulator_config();
    signer.emulators[0].1 = "127.0.0.1:0".into();
    let signer = CovenantConfig::SignerEmulation(signer);
    let serialized = serde_json::to_value(&signer).unwrap();
    assert_eq!(serialized["mode"], "signer_emulation");
    let decoded: CovenantConfig = serde_json::from_value(serialized).unwrap();
    let policy = decoded
        .get_emulator()
        .await
        .unwrap()
        .get_signer_for(hash)
        .unwrap();
    assert!(matches!(policy, sapio_base::Clause::Key(_)));
    assert!(!decoded.allows_native_ctv());

    let CovenantConfig::SignerEmulation(config) = decoded else {
        unreachable!()
    };
    let research = CovenantConfig::SignerEmulationWithNativeCtvResearch(config);
    let serialized = serde_json::to_value(&research).unwrap();
    assert_eq!(
        serialized["mode"],
        "signer_emulation_with_native_ctv_research"
    );
    let decoded: CovenantConfig = serde_json::from_value(serialized).unwrap();
    assert!(decoded.allows_native_ctv());
    assert_eq!(
        decoded
            .get_emulator()
            .await
            .unwrap()
            .get_signer_for(hash)
            .unwrap(),
        policy
    );

    let mut invalid = emulator_config();
    invalid.threshold = 0;
    assert!(CovenantConfig::SignerEmulation(invalid)
        .get_emulator()
        .await
        .is_err());
}

#[test]
fn configuration_requires_a_known_explicit_covenant_mode() {
    let original: serde_json::Value =
        serde_json::from_str(include_str!("../../../contrib/vectors/basic_config.json")).unwrap();
    let config: Config = serde_json::from_value(original.clone()).unwrap();
    assert!(matches!(
        config.active.covenant,
        CovenantConfig::NativeCtvResearch {}
    ));

    for invalid in [
        serde_json::Value::Null,
        serde_json::json!({}),
        serde_json::json!({"mode": "native_ctv"}),
        serde_json::json!({"mode": "native_ctv_research", "enabled": false}),
        serde_json::json!({"mode": "signer_emulation", "enabled": false,
            "emulators": [], "threshold": 1}),
    ] {
        let mut value = original.clone();
        value["regtest"]["covenant"] = invalid;
        assert!(
            serde_json::from_value::<Config>(value.clone()).is_err(),
            "{value}"
        );
    }
    for legacy in [
        None,
        Some(serde_json::Value::Null),
        Some(serde_json::json!({
            "enabled": false, "emulators": [], "threshold": 1
        })),
    ] {
        let mut value = original.clone();
        value["regtest"].as_object_mut().unwrap().remove("covenant");
        if let Some(legacy) = legacy {
            value["regtest"]["emulator_nodes"] = legacy;
        }
        assert!(serde_json::from_value::<Config>(value).is_err());
    }
}

#[tokio::test]
async fn wizard_requires_an_explicit_choice_and_collects_signer_configuration() {
    use tokio::io::AsyncBufReadExt;

    let mut empty = BufReader::new(&b"\n"[..]).lines();
    assert!(covenant_wizard(&mut empty).await.is_err());
    let mut native = BufReader::new(&b"native_ctv_research\n"[..]).lines();
    assert!(matches!(
        covenant_wizard(&mut native).await.unwrap(),
        CovenantConfig::NativeCtvResearch {}
    ));

    let key = emulator_config().emulators[0].0;
    let input = format!("signer_emulation\n{key}\n127.0.0.1:8367\n\n0\n2\n1\n");
    let mut signer = BufReader::new(input.as_bytes()).lines();
    let CovenantConfig::SignerEmulation(config) = covenant_wizard(&mut signer).await.unwrap()
    else {
        panic!("wizard changed the chosen mode");
    };
    assert_eq!(config.emulators, vec![(key, "127.0.0.1:8367".into())]);
    assert_eq!(config.threshold, 1);
    assert_eq!(config.request_timeout_secs, 30);

    let input = format!("signer_emulation_with_native_ctv_research\n{key}\n127.0.0.1:8367\n\n1\n");
    let mut signer = BufReader::new(input.as_bytes()).lines();
    let CovenantConfig::SignerEmulationWithNativeCtvResearch(config) =
        covenant_wizard(&mut signer).await.unwrap()
    else {
        panic!("wizard changed the chosen research mode");
    };
    assert_eq!(config.emulators, vec![(key, "127.0.0.1:8367".into())]);
    assert_eq!(config.threshold, 1);
}

fn all_networks_inactive() -> serde_json::Value {
    let original: serde_json::Value =
        serde_json::from_str(include_str!("../../../contrib/vectors/basic_config.json")).unwrap();
    let mut networks = serde_json::Map::new();
    for name in ["main", "testnet", "testnet4", "signet", "regtest"] {
        let mut config = original["regtest"].clone();
        config["active"] = false.into();
        config["api_node"]["url"] = format!("http://{name}.invalid").into();
        networks.insert(name.into(), config);
    }
    networks.into()
}

#[test]
fn every_network_can_be_selected_with_other_configurations_inactive() {
    use bitcoin::Network;

    for (name, network) in [
        ("main", Network::Bitcoin),
        ("testnet", Network::Testnet),
        ("testnet4", Network::Testnet4),
        ("signet", Network::Signet),
        ("regtest", Network::Regtest),
    ] {
        let mut value = all_networks_inactive();
        value[name]["active"] = true.into();
        let config: Config = serde_json::from_value(value).unwrap();
        assert_eq!(config.network, network);
        assert_eq!(config.active.api_node.url, format!("http://{name}.invalid"));
        let verifier = ConfigVerifier::from(config);
        assert_eq!(
            verifier
                .networks()
                .iter()
                .filter(|(_, config)| config.is_some())
                .count(),
            1
        );
        let roundtrip: Config =
            serde_json::from_value(serde_json::to_value(verifier).unwrap()).unwrap();
        assert_eq!(roundtrip.network, network);
        assert!(roundtrip.active.active);
    }
}

#[test]
fn network_selection_rejects_absent_or_multiple_active_configurations() {
    let value = all_networks_inactive();
    let verifier: ConfigVerifier = serde_json::from_value(value.clone()).unwrap();
    assert!(matches!(
        Config::try_from(verifier),
        Err(ConfigError::NoActiveConfig)
    ));
    let names = ["main", "testnet", "testnet4", "signet", "regtest"];
    for (index, first) in names.iter().enumerate() {
        for second in &names[index + 1..] {
            let mut value = value.clone();
            value[*first]["active"] = true.into();
            value[*second]["active"] = true.into();
            let verifier: ConfigVerifier = serde_json::from_value(value).unwrap();
            assert!(matches!(
                Config::try_from(verifier),
                Err(ConfigError::TooManyActiveNetworks)
            ));
        }
    }
}
