// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use bitcoin::consensus::serialize;
use bitcoin::consensus::Decodable;
use bitcoin::hashes::Hash;
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
use emulator_connect::CTVAvailable;
use emulator_connect::CTVEmulator;
use sapio::contract::Compiled;
use sapio::contract::Context;
use sapio::util::extended_address::ExtendedAddress;
use sapio_base::txindex::TxIndex;
use sapio_base::txindex::TxIndexLogger;
use sapio_base::util::CTVHash;
use sapio_wasm_plugin::host::{PluginHandle, WasmPluginHandle};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
#[deny(missing_docs)]
use tokio::io::AsyncReadExt;
use util::*;

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
                (@arg outpoint: --outpoint +takes_value "Use this specific outpoint")
                (@arg json: "JSON to Bind")
            )
            (@subcommand for_tux =>
                (about: "Translate for TUX viewer")
                (@arg psbts: --psbt "Output in PSBT format instead of tx hex.")
                (@arg finalize: --finalize "Attempt finalizing via miniscript...")
                (@arg json: "JSON to translate")
            )
            (@subcommand create =>
                (about: "create a contract to a specific UTXO")
                (@arg amount: +required "Amount to Send in BTC")
                (@group from +required =>
                    (@arg file: -f --file +takes_value {check_file} "Which Contract to Create, given a WASM Plugin file")
                    (@arg key:  -k --key +takes_value "Which Contract to Create, given a WASM Hash")
                )
                (@arg json: "JSON of args")
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
    let emulator: Arc<dyn CTVEmulator> = if let Some(emcfg) = &cfg.emulator_nodes {
        if emcfg.enabled {
            emcfg.get_emulator()?.into()
        } else {
            Arc::new(CTVAvailable)
        }
    } else {
        Arc::new(CTVAvailable)
    };
    let plugin_map = cfg.plugin_map.map(|x| {
        x.into_iter()
            .map(|(x, y)| (x.into_bytes().into(), y.into()))
            .collect()
    });
    {
        let mut emulator = emulator.clone();
        // Drop Emulator from own thread...
        std::thread::spawn(move || loop {
            if let Some(_) = Arc::get_mut(&mut emulator) {
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
            Some(("list", _args)) => {
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
                let outpoint: Option<bitcoin::OutPoint> = args
                    .value_of("outpoint")
                    .map(serde_json::from_str)
                    .transpose()?;
                let client =
                    rpc::Client::new(cfg.api_node.url.clone(), cfg.api_node.auth.clone()).await?;
                let j: Compiled = if let Some(json) = args.value_of("json") {
                    serde_json::from_str(json)?
                } else {
                    let mut s = String::new();
                    tokio::io::stdin().read_to_string(&mut s).await?;
                    serde_json::from_str(&s)?
                };

                let (tx, vout) = if use_mock {
                    let ctx = Context::new(config.network, j.amount_range.max(), emulator.clone());
                    let mut tx = ctx
                        .template()
                        .add_output(j.amount_range.max(), &j, None)?
                        .get_tx();
                    tx.input[0].previous_output = create_mock_output();
                    (tx, 0)
                } else if let Some(outpoint) = outpoint {
                    let res = client.get_raw_transaction(&outpoint.txid, None).await?;
                    (res, outpoint.vout)
                } else {
                    let mut spends = HashMap::new();
                    if let ExtendedAddress::Address(ref a) = j.address {
                        spends.insert(format!("{}", a), j.amount_range.max());
                    } else {
                        Err("Must have a valid address")?;
                    }
                    let res = client
                        .wallet_create_funded_psbt(&[], &spends, None, None, None)
                        .await?;
                    let psbt = PartiallySignedTransaction::consensus_decode(
                        &base64::decode(&res.psbt)?[..],
                    )?;
                    let tx = psbt.extract_tx();
                    // if change pos is -1, then +1%len == 0. If it is 0, then 1. If 1, then 2 % len == 0.
                    let vout = ((res.change_position + 1) as usize) % tx.output.len();
                    (tx, vout as u32)
                };
                let logger = Rc::new(TxIndexLogger::new());
                (*logger).add_tx(Arc::new(tx.clone()))?;

                let (mut txns, mut meta) = j.bind_psbt(
                    OutPoint::new(tx.txid(), vout as u32),
                    HashMap::new(),
                    logger,
                    emulator.as_ref(),
                )?;
                if outpoint.is_none() {
                    txns.push(PartiallySignedTransaction::from_unsigned_tx(tx)?);
                    meta.push(serde_json::json!({
                        "color": "black",
                        "metadata": {"label":"funding"},
                        "utxo_metadata": {}
                    }));
                }
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
                ) = if let Some(json) = args.value_of("json") {
                    serde_json::from_str(json)?
                } else {
                    let mut s = String::new();
                    tokio::io::stdin().read_to_string(&mut s).await?;
                    serde_json::from_str(&s)?
                };
                let encode_as_psbt = args.is_present("psbts");
                let finalize_psbt = args.is_present("finalize");
                let secp = Secp256k1::new();
                let program = Program {
                    program: txns
                        .into_iter()
                        .zip(metadata.into_iter())
                        .map(|(mut u, mut v)| {
                            if finalize_psbt {
                                miniscript::psbt::finalize(&mut u, &secp)
                                    .map_err(|e| println!("{:?}", e))
                                    .ok();
                            }
                            let h = if encode_as_psbt {
                                let bytes = serialize(&u);
                                base64::encode(bytes)
                            } else {
                                bitcoin::consensus::encode::serialize_hex(&u.extract_tx())
                            };
                            v.as_object_mut().map(|ref mut m| {
                                m.insert("hex".into(), h.into());
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
                            Ok(v)
                        })
                        .collect::<Result<Vec<_>, String>>()?,
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
                    &emulator,
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
                let params = if let Some(params) = args.value_of("json") {
                    serde_json::from_str(params)?
                } else {
                    let mut s = String::new();
                    tokio::io::stdin().read_to_string(&mut s).await?;
                    serde_json::from_str(&s)?
                };
                if let Err(it) = validator.validate(&params) {
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
                    &emulator,
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
                    &emulator,
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
                    &emulator,
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
