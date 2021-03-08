#[deny(missing_docs)]
use bitcoin::consensus::serialize;
use bitcoin::consensus::Decodable;
use bitcoin::hashes::{hex::ToHex, Hash};
use bitcoin::secp256k1::Secp256k1;
use bitcoin::util::bip32::ExtendedPrivKey;
use bitcoin::util::bip32::ExtendedPubKey;
use bitcoin::util::psbt::PartiallySignedTransaction;
use bitcoin::Amount;
use bitcoin::Denomination;
use bitcoin::OutPoint;
use bitcoincore_rpc_async as rpc;
use bitcoincore_rpc_async::RpcApi;
use clap::clap_app;
use config::*;
use emulator_connect::servers::hd::HDOracleEmulator;
use emulator_connect::CTVEmulator;
use emulator_connect::NullEmulator;
use sapio::contract::Compiled;
use sapio::contract::Context;
use sapio_base::txindex::TxIndex;
use sapio_base::txindex::TxIndexLogger;
use sapio_base::util::CTVHash;
use sapio_wasm_plugin::host::{PluginHandle, WasmPluginHandle};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use util::*;
use wasmer::*;

pub mod config;
mod util;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = clap_app!("sapio-cli" =>
        (@setting SubcommandRequiredElseHelp)
        (version: "0.1.0 Beta")
        (author: "Jeremy Rubin <j@rubin.io>")
        (about: "Sapio CLI for Bitcoin Smart Contracts")
        (@arg config: -c --config +takes_value #{1,2} {check_file} "Sets a custom config file")
        (@arg debug: -d ... "Sets the level of debugging information")
        (@subcommand emulator =>
            (about: "Make Requests to Emulator Servers")
            (@subcommand sign =>
                (about: "Sign a PSBT")
                (@arg psbt: -p --psbt +takes_value +required #{1,2} {check_file} "The file containing the PSBT to Sign")
                (@arg out: -o --output +takes_value +required #{1,2} {check_file_not} "The file to save the resulting PSBT")
            )
            (@subcommand get_key =>
                (about: "Get Signing Condition")
                (@arg psbt: -p --psbt +takes_value +required #{1,2} {check_file} "The file containing the PSBT to Get a Key For")
            )
            (@subcommand show =>
                (about: "Show a psbt")
                (@arg psbt: -p --psbt +takes_value +required #{1,2} {check_file} "The file containing the PSBT to Get a Key For")
            )
            (@subcommand server =>
                (about: "run an emulation server")
                (@arg sync: --sync  "Run in Synchronous mode")
                (@arg seed: +takes_value +required {check_file} "The file containing the Seed")
                (@arg interface: +required +takes_value "The Interface to Bind")
            )
        )
        (@subcommand contract =>
            (about: "Create or Manage a Contract")
            (@subcommand bind =>
                (about: "Bind Contract to a specific UTXO")
                (@arg mock: --mock "Create a fake output for this txn.")
                (@arg json: +required "JSON to Bind")
            )
            (@subcommand for_tux =>
                (about: "Translate for TUX viewer")
                (@arg json: +required "JSON to translate")
            )
            (@subcommand create =>
                (about: "create a contract to a specific UTXO")
                (@arg amount: +required "Amount to Send in BTC")
                (@group from +required =>
                    (@arg file: -f --file +takes_value {check_file} "Which Contract to Create, given a WASM Plugin file")
                    (@arg key:  -k --key +takes_value "Which Contract to Create, given a WASM Hash")
                )
                (@arg params: +required "JSON of args")
            )
            (@subcommand load =>
                (about: "Load a wasm contract module, returns the hex sha3 hash key")
                (@arg file: -f --file +required +takes_value {check_file} "Which Contract to Create, given a WASM Plugin file")
            )
            (@subcommand api =>
                (about: "Machine Readable API for a plugin, pipe into jq for pretty formatting.")
                (@group from +required =>
                    (@arg file: -f --file +takes_value {check_file} "Which Contract to Create, given a WASM Plugin file")
                    (@arg key:  -k --key +takes_value "Which Contract to Create, given a WASM Hash")
                )
            )
            (@subcommand info =>
                (about: "View human readable basic information for a plugin")
                (@group from +required =>
                    (@arg file: -f --file +takes_value {check_file} "Which Contract to Create, given a WASM Plugin file")
                    (@arg key:  -k --key +takes_value "Which Contract to Create, given a WASM Hash")
                )
            )
            (@subcommand list =>
                (about: "list available contracts")
            )
        )
    );
    let matches = app.get_matches();

    let config = Config::setup(&matches, "org", "judica", "sapio-cli").await?;

    let cfg = config.active;
    let emulator = NullEmulator(if let Some(emcfg) = &cfg.emulator_nodes {
        if emcfg.enabled {
            Some(emcfg.get_emulator()?.into())
        } else {
            None
        }
    } else {
        None
    });
    let plugin_map = cfg.plugin_map.map(|x| {
        x.into_iter()
            .map(|(x, y)| (x.into_bytes().into(), y.into()))
            .collect()
    });
    {
        let mut emulator = emulator.clone();
        // Drop Emulator from own thread...
        std::thread::spawn(move || loop {
            if let Some(_) = emulator.0.as_mut().and_then(|e| Arc::get_mut(e)) {
                break;
            }
        });
    }
    match matches.subcommand() {
        Some(("emulator", sign_matches)) => match sign_matches.subcommand() {
            Some(("sign", args)) => {
                let psbt = decode_psbt_file(args, "psbt")?;
                let psbt = emulator.sign(psbt)?;
                let bytes = serialize(&psbt);
                std::fs::write(args.value_of_os("out").unwrap(), &base64::encode(bytes))?;
            }
            Some(("get_key", args)) => {
                let psbt = decode_psbt_file(args, "psbt")?;
                let h = emulator.get_signer_for(psbt.extract_tx().get_ctv_hash(0))?;
                println!("{}", h);
            }
            Some(("show", args)) => {
                let psbt = decode_psbt_file(args, "psbt")?;
                println!("{:?}", psbt);
            }
            Some(("server", args)) => {
                let filename = args.value_of("seed").unwrap();
                let contents = tokio::fs::read(filename).await?;

                let root = ExtendedPrivKey::new_master(config.network, &contents[..]).unwrap();
                let pk_root = ExtendedPubKey::from_private(&Secp256k1::new(), &root);
                let oracle = HDOracleEmulator::new(root, args.is_present("sync"));
                let server = oracle.bind(args.value_of("interface").unwrap());
                println!("Running Oracle With Key: {}", pk_root);
                server.await?;
            }
            _ => unreachable!(),
        },
        Some(("contract", matches)) => match matches.subcommand() {
            Some(("list", args)) => {
                let plugins = WasmPluginHandle::load_all_keys(
                    "org".into(),
                    "judica".into(),
                    "sapio-cli".into(),
                    emulator,
                    config.network,
                    plugin_map,
                )
                .await?;
                for plugin in plugins {
                    println!("{} -- {}", plugin.get_name()?, plugin.id().to_string());
                }
            }
            Some(("bind", args)) => {
                fn create_mock_output() -> bitcoin::OutPoint {
                    bitcoin::OutPoint {
                        txid: bitcoin::hashes::sha256d::Hash::from_inner(
                            bitcoin::hashes::sha256::Hash::hash(format!("mock:{}", 0).as_bytes())
                                .into_inner(),
                        )
                        .into(),
                        vout: 0,
                    }
                }

                let use_mock = args.is_present("mock");
                let client =
                    rpc::Client::new(cfg.api_node.url.clone(), cfg.api_node.auth.clone()).await?;
                let j: Compiled = serde_json::from_str(args.value_of("json").unwrap())?;

                let em = Arc::new(emulator);
                let (tx, vout) = if use_mock {
                    let ctx = Context::new(config.network, j.amount_range.max(), Some(em.clone()));
                    let mut tx = ctx
                        .template()
                        .add_output(j.amount_range.max(), &j, None)?
                        .get_tx();
                    tx.input[0].previous_output = create_mock_output();
                    (tx, 0)
                } else {
                    let mut spends = HashMap::new();
                    spends.insert(format!("{}", j.address), j.amount_range.max());
                    let res = client
                        .wallet_create_funded_psbt(&[], &spends, None, None, None)
                        .await?;
                    let psbt = PartiallySignedTransaction::consensus_decode(
                        &base64::decode(&res.psbt)?[..],
                    )?;
                    let tx = psbt.extract_tx();
                    // if change pos is -1, then +1%len == 0. If it is 0, then 1. If 1, then 2 % len == 0.
                    let vout = ((res.change_position + 1) as usize) % tx.output.len();
                    (tx, vout)
                };
                let logger = Rc::new(TxIndexLogger::new());
                (*logger).add_tx(Arc::new(tx.clone()))?;

                let (mut txns, mut meta) = j.bind_psbt(
                    OutPoint::new(tx.txid(), vout as u32),
                    HashMap::new(),
                    logger,
                    em.as_ref(),
                )?;
                txns.push(PartiallySignedTransaction::from_unsigned_tx(tx)?);
                meta.push(serde_json::json!({
                    "color": "black",
                    "metadata": {"label":"funding"},
                    "utxo_metadata": {}
                }));
                println!("{}", serde_json::to_string_pretty(&(txns, meta))?);
            }
            Some(("for_tux", args)) => {
                use serde::{Deserialize, Serialize};
                /// A `Program` is a wrapper type for a list of
                /// JSON objects that should be of form:
                /// ```json
                /// {
                ///     "hex" : Hex Encoded Transaction
                ///     "color" : HTML Color,
                ///     "metadata" : JSON Value,
                ///     "utxo_metadata" : {
                ///         "key" : "value",
                ///         ...
                ///     }
                /// }
                /// ```
                #[derive(Serialize, Deserialize, Debug)]
                pub struct Program {
                    program: Vec<serde_json::Value>,
                }
                let (txns, metadata): (
                    Vec<bitcoin::util::psbt::PartiallySignedTransaction>,
                    Vec<serde_json::Value>,
                ) = serde_json::from_str(args.value_of("json").unwrap())?;
                let program = Program {
                    program: txns
                        .into_iter()
                        .map(|p| p.extract_tx())
                        .map(|u| bitcoin::consensus::encode::serialize(&u))
                        .zip(metadata.into_iter())
                        .map(|(h, mut v)| {
                            v.as_object_mut().map(|ref mut m| {
                                m.insert("hex".into(), h.to_hex().into());
                                m.insert(
                                    "label".into(),
                                    m.get("metadata")
                                        .unwrap()
                                        .as_object()
                                        .unwrap()
                                        .get("label")
                                        .unwrap_or(&serde_json::json!("unlabeled"))
                                        .clone(),
                                )
                            });
                            v
                        })
                        .collect(),
                };
                println!("{}", serde_json::to_string_pretty(&program)?);
            }
            Some(("create", args)) => {
                let amt =
                    Amount::from_str_in(args.value_of("amount").unwrap(), Denomination::Bitcoin)?;
                let sph = WasmPluginHandle::new(
                    "org".into(),
                    "judica".into(),
                    "sapio-cli".into(),
                    emulator,
                    args.value_of("key"),
                    args.value_of_os("file"),
                    config.network,
                    plugin_map,
                )
                .await?;
                let api = sph.get_api()?;
                let validator = jsonschema_valid::Config::from_schema(
                    &api,
                    Some(jsonschema_valid::schemas::Draft::Draft6),
                )?;
                let params = args.value_of("params").unwrap();
                if let Err(it) = validator.validate(&serde_json::from_str(params)?) {
                    for err in it {
                        println!("Error: {}", err);
                    }
                    return Ok(());
                }
                let create_args =
                    sapio_wasm_plugin::CreateArgs(params.to_string(), config.network, amt);
                let v = sph.create(&create_args)?;
                println!("{}", serde_json::to_string(&v)?);
            }
            Some(("api", args)) => {
                let sph = WasmPluginHandle::new(
                    "org".into(),
                    "judica".into(),
                    "sapio-cli".into(),
                    emulator,
                    args.value_of("key"),
                    args.value_of_os("file"),
                    config.network,
                    plugin_map,
                )
                .await?;
                println!("{}", sph.get_api()?);
            }
            Some(("info", args)) => {
                let sph = WasmPluginHandle::new(
                    "org".into(),
                    "judica".into(),
                    "sapio-cli".into(),
                    emulator,
                    args.value_of("key"),
                    args.value_of_os("file"),
                    config.network,
                    plugin_map,
                )
                .await?;
                println!("Name: {}", sph.get_name()?);
                let api = sph.get_api()?;
                println!(
                    "Description:\n{}",
                    api.get("description").unwrap().as_str().unwrap()
                );
                println!("Parameters:");
                for (i, param) in api
                    .get("properties")
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .keys()
                    .enumerate()
                {
                    println!("{}. {}", i, param)
                }
            }
            Some(("load", args)) => {
                let sph = WasmPluginHandle::new(
                    "org".into(),
                    "judica".into(),
                    "sapio-cli".into(),
                    emulator,
                    None,
                    args.value_of_os("file"),
                    config.network,
                    plugin_map,
                )
                .await?;
                println!("{}", sph.id().to_string());
            }
            _ => unreachable!(),
        },
        _ => unreachable!(),
    };

    Ok(())
}
