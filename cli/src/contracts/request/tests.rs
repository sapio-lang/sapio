use super::*;
use bitcoin::consensus::encode::serialize_hex;
use bitcoin::psbt::raw::Key;
use bitcoin::secp256k1::{Message, Secp256k1, SecretKey};
use bitcoin::{Address, EcdsaSighashType, Network, PublicKey, ScriptBuf, Transaction, TxIn, TxOut};
use emulator_connect::CTVAvailable;
use sapio::contract::abi::studio::SapioStudioFormat;
use sapio_base::covenant::{Ctv, LoweringPlan};
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn studio_context_requires_an_explicit_covenant_mode() {
    let original = serde_json::json!({
        "path": ".",
        "covenant": {"mode": "native_ctv_research"},
        "module_locator": null,
        "net": Network::Regtest,
        "plugin_map": null
    });
    let context: Common = serde_json::from_value(original.clone()).unwrap();
    assert!(matches!(
        context.covenant,
        CovenantConfig::NativeCtvResearch {}
    ));
    for legacy in [
        None,
        Some(serde_json::Value::Null),
        Some(serde_json::json!({
            "enabled": false, "emulators": [], "threshold": 1
        })),
    ] {
        let mut value = original.clone();
        value.as_object_mut().unwrap().remove("covenant");
        if let Some(legacy) = legacy {
            value["emulator"] = legacy;
        }
        assert!(serde_json::from_value::<Common>(value).is_err());
    }
    let mut null = original;
    null["covenant"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<Common>(null).is_err());
}

#[tokio::test]
async fn listing_modules_does_not_resolve_or_validate_unused_signer_connections() {
    let CovenantConfig::SignerEmulation(mut config) = signer_config() else {
        unreachable!()
    };
    config.threshold = 0;
    config.emulators[0].1 = "address-without-a-port".into();
    let response = Request {
        context: Common {
            path: std::env::temp_dir()
                .join(format!("sapio-empty-module-list-{}", rand::random::<u64>())),
            covenant: CovenantConfig::SignerEmulation(config),
            module_locator: None,
            net: Network::Regtest,
            plugin_map: None,
        },
        command: Command::List(List),
    }
    .handle_inner()
    .await
    .unwrap();
    let CommandReturn::List(list) = response else {
        panic!("expected module list")
    };
    assert!(list.items.is_empty());
}

struct TestSigner {
    lowering: LoweringPlan,
    key: bitcoin::XOnlyPublicKey,
    signatures: AtomicUsize,
}

impl TestSigner {
    fn new() -> Arc<Self> {
        let CovenantConfig::SignerEmulation(config) = signer_config() else {
            unreachable!()
        };
        Arc::new(Self {
            lowering: LoweringPlan::CtvEmulation {
                signers: config.emulators.iter().map(|(key, _)| *key).collect(),
                threshold: config.threshold,
            },
            key: bitcoin::XOnlyPublicKey::from_str(
                "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            )
            .unwrap(),
            signatures: AtomicUsize::new(0),
        })
    }
}

impl CTVEmulator for TestSigner {
    fn get_signer_for(
        &self,
        hash: bitcoin::hashes::sha256::Hash,
    ) -> Result<sapio_base::Clause, emulator_connect::EmulatorError> {
        self.lowering
            .lower_ctv(Ctv(hash))
            .map_err(|error| std::io::Error::other(error).into())
    }

    fn sign(&self, psbt: Psbt) -> Result<Psbt, emulator_connect::EmulatorError> {
        self.signatures.fetch_add(1, Ordering::SeqCst);
        Ok(psbt)
    }
}

fn signer_config() -> CovenantConfig {
    let secret = bitcoin::bip32::Xpriv::new_master(Network::Regtest, &[1; 32]).unwrap();
    let root = bitcoin::bip32::Xpub::from_priv(&Secp256k1::new(), &secret);
    CovenantConfig::SignerEmulation(crate::config::EmulatorConfig {
        emulators: vec![(root, "127.0.0.1:0".into())],
        threshold: 1,
        request_timeout_secs: 30,
    })
}

struct Payment;

impl Payment {
    #[sapio::then]
    fn pay(self, ctx: Context) {
        ctx.template()
            .add_output(bitcoin::Amount::from_sat(1_000), &contract(), None)?
            .into()
    }
}

impl sapio::contract::Contract for Payment {
    sapio::declare! {actions, Self::pay}
}

fn payment(lowering: LoweringPlan) -> Compiled {
    use sapio::contract::Compilable;
    Payment
        .compile(Context::new(
            Network::Regtest,
            bitcoin::Amount::from_sat(1_000),
            lowering,
            "payment".try_into().unwrap(),
            Arc::new(MapEffectDB::default()),
            None,
        ))
        .unwrap()
}

struct MixedPayment {
    hash: bitcoin::hashes::sha256::Hash,
}

impl MixedPayment {
    #[sapio::guard(cached)]
    fn native(self) {
        sapio_base::Clause::TxTemplate(self.hash)
    }

    #[sapio::then(guarded_by = "[Self::native]")]
    fn pay(self, ctx: Context) {
        ctx.template()
            .add_output(bitcoin::Amount::from_sat(1_000), &contract(), None)?
            .into()
    }
}

impl sapio::contract::Contract for MixedPayment {
    sapio::declare! {actions, Self::pay}
}

#[tokio::test]
async fn mixed_signer_and_native_policy_requires_both_explicit_assumptions() {
    use sapio::contract::abi::object::ObjectError;
    use sapio::contract::Compilable;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    let signer = TestSigner::new();
    let hash = *payment(LoweringPlan::Native)
        .ctv_to_tx
        .keys()
        .next()
        .unwrap();
    let compiled = MixedPayment { hash }
        .compile(Context::new(
            Network::Regtest,
            bitcoin::Amount::from_sat(1_000),
            signer.lowering.clone(),
            "mixed".try_into().unwrap(),
            Arc::new(MapEffectDB::default()),
            None,
        ))
        .unwrap();
    compiled.validate_for_emulator(signer.as_ref()).unwrap();
    assert!(compiled.requires_native_ctv());

    let mut ordinary = request(compiled.clone(), &[]);
    ordinary.use_txn = None;
    ordinary.client_url = "not a URL".into();
    let error = ordinary
        .call(Network::Regtest, signer.clone(), &signer_config())
        .await
        .unwrap_err();
    assert!(error
        .downcast_ref::<RequestError>()
        .unwrap()
        .0
        .as_str()
        .unwrap()
        .contains("requires native CTV"));

    let mut native = request(compiled.clone(), &[]);
    native.use_txn = None;
    native.client_url = "not a URL".into();
    let error = native
        .call(
            Network::Regtest,
            Arc::new(CTVAvailable),
            &CovenantConfig::NativeCtvResearch {},
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<ObjectError>(),
        Some(ObjectError::CovenantPolicyMismatch { .. })
    ));

    // A small RPC endpoint proves the matching mixed mode reaches wallet funding.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let wallet = tokio::spawn(async move {
        tokio::time::timeout(std::time::Duration::from_secs(5), async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let mut length = None;
            loop {
                let mut line = String::new();
                assert_ne!(stream.read_line(&mut line).await.unwrap(), 0);
                if line == "\r\n" { break; }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = Some(value.trim().parse::<usize>().unwrap());
                }
            }
            let mut body = vec![0; length.unwrap()];
            stream.read_exact(&mut body).await.unwrap();
            let request: Value = serde_json::from_slice(&body).unwrap();
            let body = serde_json::json!({
                "result": null,
                "error": {"code": -1, "message": "wallet funding boundary reached"},
                "id": request["id"]
            }).to_string();
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.get_mut().write_all(response.as_bytes()).await.unwrap();
                request
        }).await.unwrap()
    });
    let CovenantConfig::SignerEmulation(config) = signer_config() else {
        unreachable!()
    };
    let research = CovenantConfig::SignerEmulationWithNativeCtvResearch(config);
    let expected_address =
        Address::from_script(&ScriptBuf::from(&compiled.address), Network::Regtest)
            .unwrap()
            .to_string();
    let expected_amount = compiled.required_input_amount.to_btc();
    let mut bind = request(compiled, &[]);
    bind.use_txn = None;
    bind.client_url = format!("http://{address}");
    let error = bind
        .call(Network::Regtest, signer.clone(), &research)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("wallet funding boundary reached"),
        "{error}"
    );
    let funding_request = wallet.await.unwrap();
    assert_eq!(funding_request["method"], "walletcreatefundedpsbt");
    assert_eq!(funding_request["params"][0], serde_json::json!([]));
    assert_eq!(
        funding_request["params"][1],
        serde_json::json!({(expected_address): expected_amount})
    );
    assert_eq!(signer.signatures.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn backend_mismatches_fail_before_wallet_funding_or_signing() {
    use sapio::contract::abi::object::ObjectError;

    let signer = TestSigner::new();
    let cases: [(Compiled, Arc<dyn CTVEmulator>, CovenantConfig); 2] = [
        (
            payment(LoweringPlan::Native),
            signer.clone(),
            signer_config(),
        ),
        (
            payment(signer.lowering.clone()),
            Arc::new(CTVAvailable),
            CovenantConfig::NativeCtvResearch {},
        ),
    ];
    for (compiled, emulator, covenant) in cases {
        let mut bind = request(compiled, &[]);
        bind.use_txn = None;
        // Any attempt to construct the RPC client or fund through it is an error.
        bind.client_url = "not a URL".into();
        let error = bind
            .call(Network::Regtest, emulator, &covenant)
            .await
            .unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<ObjectError>(),
                Some(ObjectError::CovenantPolicyMismatch { .. })
            ),
            "{error}"
        );
    }
    assert_eq!(signer.signatures.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn raw_native_checks_require_the_research_mode_before_funding() {
    use bitcoin::blockdata::opcodes::all::{OP_DROP, OP_NOP4};
    use sapio::contract::abi::object::{RawTaproot, SupportedDescriptors};

    let signer = TestSigner::new();
    let script = bitcoin::blockdata::script::Builder::new()
        .push_slice([1; 32])
        .push_opcode(OP_NOP4)
        .push_opcode(OP_DROP)
        .push_int(1)
        .into_script();
    let raw = RawTaproot::new(signer.key, vec![(0, script)]).unwrap();
    let mut compiled = Compiled::from_address(
        Address::from_script(&raw.script_pubkey(), Network::Regtest).unwrap(),
        bitcoin::Amount::ZERO,
    );
    compiled.descriptor = Some(SupportedDescriptors::Taproot(raw));
    compiled.validate_for_emulator(signer.as_ref()).unwrap();

    let mut bind = request(compiled.clone(), &[]);
    bind.use_txn = None;
    bind.client_url = "not a URL".into();
    let error = bind
        .call(Network::Regtest, signer.clone(), &signer_config())
        .await
        .unwrap_err();
    let error = error.downcast_ref::<RequestError>().unwrap();
    assert!(error.0.as_str().unwrap().contains("requires native CTV"));
    assert_eq!(signer.signatures.load(Ordering::SeqCst), 0);

    let psbt = funding_psbt(&compiled);
    request(compiled, &psbt.serialize())
        .call(
            Network::Regtest,
            Arc::new(CTVAvailable),
            &CovenantConfig::NativeCtvResearch {},
        )
        .await
        .unwrap();
}

fn contract() -> Compiled {
    Compiled::from_address(
        Address::from_str("bcrt1qumrrqgt7e3a7damzm8x97m6sjs20u8hjw2hcjj")
            .unwrap()
            .require_network(Network::Regtest)
            .unwrap(),
        bitcoin::Amount::ZERO,
    )
}

fn funding_psbt(compiled: &Compiled) -> Psbt {
    let tx = Transaction {
        version: bitcoin::transaction::Version(2),
        lock_time: bitcoin::absolute::LockTime::from_consensus(0),
        input: vec![TxIn::default()],
        output: vec![
            TxOut {
                value: bitcoin::Amount::from_sat(500),
                script_pubkey: ScriptBuf::new(),
            },
            TxOut {
                value: bitcoin::Amount::from_sat(1_000),
                script_pubkey: (&compiled.address).into(),
            },
        ],
    };
    let mut psbt = Psbt::from_unsigned_tx(tx).unwrap();
    psbt.inputs[0].witness_utxo = Some(TxOut {
        value: bitcoin::Amount::from_sat(2_000),
        script_pubkey: ScriptBuf::new(),
    });
    let unknown = |key| Key {
        type_value: 0xfa,
        key: vec![key],
    };
    psbt.unknown.insert(unknown(1), vec![2]);
    psbt.inputs[0].unknown.insert(unknown(3), vec![4]);
    psbt.outputs[1].unknown.insert(unknown(5), vec![6]);
    psbt
}

fn request(compiled: Compiled, psbt: &[u8]) -> Bind {
    Bind {
        client_url: "http://127.0.0.1:1".into(),
        client_auth: rpc::Auth::None,
        use_base64: true,
        use_mock: false,
        outpoint: None,
        use_txn: Some(base64::encode(psbt)),
        compiled,
        ordinals_info: None,
    }
}

fn assert_preserved_funding(bound: &Program, compiled: &Compiled, expected: &Psbt) {
    let expected_tx = expected.clone().extract_tx().unwrap();
    assert_eq!(
        bound.program.get(&compiled.root_path).unwrap().out,
        OutPoint::new(expected_tx.compute_txid(), 1)
    );
    let funding = bound
        .program
        .values()
        .flat_map(|object| &object.txs)
        .find(|entry| {
            let SapioStudioFormat::LinkedPSBT { metadata, .. } = entry;
            metadata.label.as_deref() == Some("funding")
        })
        .unwrap();
    let SapioStudioFormat::LinkedPSBT { psbt, hex, .. } = funding;
    let restored: Psbt = Psbt::deserialize(&base64::decode(psbt).unwrap()).unwrap();
    assert_eq!(restored, *expected);
    assert_eq!(*hex, serialize_hex(&expected_tx));
}

#[tokio::test]
async fn supplied_funding_keeps_partial_signatures_and_all_psbt_maps() {
    let compiled = contract();
    let mut psbt = funding_psbt(&compiled);
    let secp = Secp256k1::new();
    let secret = SecretKey::from_slice(&[1; 32]).unwrap();
    let key = PublicKey::new(bitcoin::secp256k1::PublicKey::from_secret_key(
        &secp, &secret,
    ));
    let sig = bitcoin::ecdsa::Signature {
        signature: secp.sign_ecdsa(&Message::from_digest_slice(&[2; 32]).unwrap(), &secret),
        sighash_type: EcdsaSighashType::All,
    };
    psbt.inputs[0].partial_sigs.insert(key, sig);
    let bound = request(compiled.clone(), &psbt.serialize())
        .call(
            Network::Regtest,
            Arc::new(CTVAvailable),
            &CovenantConfig::NativeCtvResearch {},
        )
        .await
        .unwrap();
    assert_preserved_funding(&bound, &compiled, &psbt);
}

#[tokio::test]
async fn finalized_legacy_funding_keeps_its_psbt_and_binds_the_extracted_txid() {
    let compiled = contract();
    let mut psbt = funding_psbt(&compiled);
    psbt.inputs[0].final_script_sig = Some(ScriptBuf::from(vec![1, 0x42]));
    assert_ne!(
        psbt.unsigned_tx.compute_txid(),
        psbt.clone().extract_tx().unwrap().compute_txid()
    );
    let bound = request(compiled.clone(), &psbt.serialize())
        .call(
            Network::Regtest,
            Arc::new(CTVAvailable),
            &CovenantConfig::NativeCtvResearch {},
        )
        .await
        .unwrap();
    assert_preserved_funding(&bound, &compiled, &psbt);
}

#[tokio::test]
async fn zero_input_funding_psbt_returns_a_validation_error() {
    let compiled = contract();
    let mut psbt = funding_psbt(&compiled);
    psbt.unsigned_tx.input.clear();
    psbt.inputs.clear();
    let error = request(compiled, &psbt.serialize())
        .call(
            Network::Regtest,
            Arc::new(CTVAvailable),
            &CovenantConfig::NativeCtvResearch {},
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<sapio_psbt::PSBTValidationError>(),
        Some(sapio_psbt::PSBTValidationError::NoInputs)
    ));
}

#[tokio::test]
async fn supplied_funding_rejects_trailing_psbt_bytes() {
    let compiled = contract();
    let mut bytes = funding_psbt(&compiled).serialize();
    bytes.push(0);
    assert!(request(compiled, &bytes)
        .call(
            Network::Regtest,
            Arc::new(CTVAvailable),
            &CovenantConfig::NativeCtvResearch {}
        )
        .await
        .is_err());
}

#[test]
fn funding_output_uses_the_contract_script_and_rejects_missing_outputs() {
    let compiled = contract();
    let script = bitcoin::ScriptBuf::from(&compiled.address);
    let mut psbt = funding_psbt(&compiled);
    assert_eq!(funding_output(&psbt, &script).unwrap(), 1);
    psbt.unsigned_tx.output.swap(0, 1);
    psbt.outputs.swap(0, 1);
    assert_eq!(funding_output(&psbt, &script).unwrap(), 0);

    psbt.unsigned_tx.output[0].script_pubkey = ScriptBuf::new();
    assert!(funding_output(&psbt, &script)
        .unwrap_err()
        .is::<RequestError>());
    psbt.unsigned_tx.output.clear();
    psbt.outputs.clear();
    assert!(funding_output(&psbt, &script)
        .unwrap_err()
        .is::<RequestError>());
}

#[test]
fn funding_output_validates_psbt_maps_before_extraction() {
    use sapio_psbt::PSBTValidationError;

    let compiled = contract();
    let script = bitcoin::ScriptBuf::from(&compiled.address);
    let mut psbt = funding_psbt(&compiled);
    psbt.inputs.clear();
    assert_eq!(
        funding_output(&psbt, &script)
            .unwrap_err()
            .downcast_ref::<PSBTValidationError>(),
        Some(&PSBTValidationError::InputMapCount {
            transaction: 1,
            maps: 0,
        })
    );

    let mut psbt = funding_psbt(&compiled);
    psbt.outputs.pop();
    assert_eq!(
        funding_output(&psbt, &script)
            .unwrap_err()
            .downcast_ref::<PSBTValidationError>(),
        Some(&PSBTValidationError::OutputMapCount {
            transaction: 2,
            maps: 1,
        })
    );
}

#[test]
fn rpc_funding_must_match_the_requested_txid_and_output_index() {
    let tx = funding_psbt(&contract()).extract_tx().unwrap();
    let requested = OutPoint::new(tx.compute_txid(), 1);
    validate_funding_outpoint(&tx, requested).unwrap();
    assert!(matches!(
        validate_funding_outpoint(&tx, OutPoint::new(tx.compute_txid(), 2)),
        Err(TxIndexError::IndexTooHigh(2))
    ));

    let mut wrong = tx;
    wrong.output[1].value += bitcoin::Amount::ONE_SAT;
    for vout in [1, 2] {
        assert!(matches!(
            validate_funding_outpoint(&wrong, OutPoint { vout, ..requested }),
            Err(TxIndexError::TxidMismatch { expected, actual })
                if expected == requested.txid && actual == wrong.compute_txid()
        ));
    }
}

#[tokio::test]
async fn mock_funding_still_includes_a_funding_psbt() {
    let compiled = contract();
    let mut bind = request(compiled.clone(), &funding_psbt(&compiled).serialize());
    bind.use_txn = None;
    bind.use_mock = true;
    let bound = bind
        .call(
            Network::Regtest,
            Arc::new(CTVAvailable),
            &CovenantConfig::NativeCtvResearch {},
        )
        .await
        .unwrap();
    let funding = bound
        .program
        .values()
        .flat_map(|object| &object.txs)
        .next()
        .unwrap();
    let SapioStudioFormat::LinkedPSBT { psbt, .. } = funding;
    let psbt: Psbt = Psbt::deserialize(&base64::decode(psbt).unwrap()).unwrap();
    sapio_psbt::validate_psbt(&psbt).unwrap();
    assert_eq!(
        psbt.unsigned_tx.input[0].previous_output,
        create_mock_output()
    );
    assert_eq!(psbt.unsigned_tx.output.len(), 1);
    assert_eq!(
        psbt.unsigned_tx.output[0].script_pubkey,
        bitcoin::ScriptBuf::from(&compiled.address)
    );
    assert_eq!(
        bound.program.get(&compiled.root_path).unwrap().out,
        OutPoint::new(psbt.unsigned_tx.compute_txid(), 0)
    );
}

#[tokio::test]
async fn funding_entry_cannot_overwrite_a_contract_named_funding() {
    for root in ["funding", "funding/@funding"] {
        let mut compiled = contract();
        compiled.root_path = SArc(Arc::new(root.try_into().unwrap()));
        let psbt = funding_psbt(&compiled);
        let bound = request(compiled.clone(), &psbt.serialize())
            .call(
                Network::Regtest,
                Arc::new(CTVAvailable),
                &CovenantConfig::NativeCtvResearch {},
            )
            .await
            .unwrap();
        assert_eq!(bound.program.len(), 2);
        assert_preserved_funding(&bound, &compiled, &psbt);
        let contract_node = bound.program.get(&compiled.root_path).unwrap();
        assert_eq!(
            contract_node.source_path.as_ref(),
            Some(&compiled.root_path)
        );
        let funding_path = SArc(Arc::new(format!("{root}/@funding").try_into().unwrap()));
        let funding = bound.program.get(&funding_path).unwrap();
        assert!(funding.source_path.is_none());
        let restored: Program =
            serde_json::from_value(serde_json::to_value(&bound).unwrap()).unwrap();
        assert!(restored
            .program
            .get(&funding_path)
            .unwrap()
            .source_path
            .is_none());
    }
}

#[tokio::test]
async fn supplied_funding_must_pass_checked_extraction_before_binding() {
    let compiled = contract();
    for missing_utxo in [false, true] {
        let mut psbt = funding_psbt(&compiled);
        if missing_utxo {
            psbt.inputs[0].witness_utxo = None;
        } else {
            psbt.unsigned_tx.output[0].value = bitcoin::Amount::from_sat(2_001);
        }
        let error = request(compiled.clone(), &psbt.serialize())
            .call(
                Network::Regtest,
                Arc::new(CTVAvailable),
                &CovenantConfig::NativeCtvResearch {},
            )
            .await
            .unwrap_err();
        assert!(error.is::<bitcoin::psbt::ExtractTxError>(), "{error}");
    }
}
