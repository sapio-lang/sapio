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
use bitcoin::OutPoint;
use bitcoincore_rpc_async as rpc;
use bitcoincore_rpc_async::RpcApi;
use clap::clap_app;
use config::*;
use emulator_connect::servers::hd::HDOracleEmulator;
use emulator_connect::CTVAvailable;
use emulator_connect::CTVEmulator;
use miniscript::psbt::PsbtExt;
use sapio::contract::context::MapEffectDB;
use sapio::contract::object::LinkedPSBT;
use sapio::contract::object::ObjectMetadata;
use sapio::contract::object::SapioStudioObject;
use sapio::contract::Compiled;
use sapio::contract::Context;
use sapio::template::output::OutputMeta;
use sapio::template::TemplateMetadata;
use sapio::util::extended_address::ExtendedAddress;
use sapio_base::effects::PathFragment;
use sapio_base::serialization_helpers::SArc;
use sapio_base::txindex::TxIndex;
use sapio_base::txindex::TxIndexLogger;
use sapio_base::util::CTVHash;
use sapio_wasm_plugin::host::{PluginHandle, WasmPluginHandle};
use sapio_wasm_plugin::CreateArgs;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::convert::TryInto;
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
    (@arg config: -c --config +takes_value #{1,1} {check_file} "Sets a custom config file")
    (@arg debug: -d ... "Sets the level of debugging information")
    (@subcommand configure =>
     (@setting SubcommandRequiredElseHelp)
     (about: "Helper to check current configuration settings")
     (@subcommand files =>
      (about: "Show where the configure files live.")
      (@arg json: -j --json "Print or return JSON formatted dirs")
     )
     (@subcommand show =>
     (about: "Print out the currently loaded configuration")
    )
     (@subcommand wizard =>
      (@arg write: -w --write "Write the default config file to the standard location.")
     (about: "Interactive wizard to create a configuration")
     )
    )
    (@subcommand signer =>
     (@setting SubcommandRequiredElseHelp)
     (about: "Make Requests to Emulator Servers")
     (@subcommand sign =>
      (about: "Sign a PSBT")
      (@arg input: -k --key +takes_value +required #{1,2} {check_file} "The file to read the key from")
      (@arg psbt: -p --psbt +takes_value  #{1,2} {check_file} "The file containing the PSBT to Sign")
      (@arg out: -o --output +takes_value  #{1,2} {check_file_not} "The file to save the resulting PSBT")
     )
     (@subcommand new =>
      (about: "Get a new xpriv")
      (@arg network: -n --network +takes_value +required #{1,2}  "One of: signet, testnet, regtest, bitcoin")
      (@arg out: -o --output +takes_value +required #{1,2} {check_file_not} "The file to save the resulting key")
     )
     (@subcommand show =>
      (about: "Show xpub for file")
      (@arg input: -i --input +takes_value +required #{1,2} {check_file} "The file to read the key from")
     )
    )
    (@subcommand emulator =>
     (@setting SubcommandRequiredElseHelp)
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
     (@subcommand psbt =>
      (@setting SubcommandRequiredElseHelp)
      (about: "Perform operations on PSBTs")
      (@subcommand finalize =>
       (about: "finalize and extract this psbt to transaction hex")
       (@arg psbt: --psbt +takes_value "psbt as base64, otherwise read from stdin")
      )
     )
     (@subcommand contract =>
      (@setting SubcommandRequiredElseHelp)
      (about: "Create or Manage a Contract")
      (@subcommand bind =>
       (about: "Bind Contract to a specific UTXO")
       (@arg base64_psbt: --base64_psbt "Output as a base64 PSBT")
       (@group from  =>
            (@arg outpoint: --outpoint +takes_value "Use this specific outpoint")
            (@arg txn: --txn +takes_value "Use this specific transaction ")
            (@arg mock: --mock "Create a fake output for this txn.")
       )
       (@arg json: "JSON to Bind")
      )
      (@subcommand create =>
       (about: "create a contract to a specific UTXO")
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
      (@subcommand logo =>
       (about: "base64 encoded png image")
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
    if let Some(("configure", config_matches)) = matches.subcommand() {
        if let Some(("wizard", args)) = config_matches.subcommand() {
            let config = ConfigVerifier::wizard().await?;
            if args.is_present("write") {
                let proj = directories::ProjectDirs::from("org", "judica", "sapio-cli")
                    .expect("Failed to find config directory");
                let path = proj.config_dir();
                tokio::fs::create_dir_all(path).await?;
                let mut pb = path.to_path_buf();
                pb.push("config.json");
                tokio::fs::write(&pb, &serde_json::to_string_pretty(&config)?).await?;
            } else {
                println!(
                    "Please write this to the config file location (see sapio-cli configure files)"
                );
            }
            return Ok(());
        }
    }

    let config = Config::setup(&matches, "org", "judica", "sapio-cli").await?;

    match matches.subcommand() {
        Some(("configure", config_matches)) => match config_matches.subcommand() {
            Some(("files", args)) => {
                let proj = directories::ProjectDirs::from("org", "judica", "sapio-cli")
                    .expect("Failed to find config directory");
                let path = proj.config_dir();
                let mut config_json = path.to_path_buf();
                config_json.push("config.json");
                let mut modules = path.to_path_buf();
                modules.push("modules");
                if args.is_present("json") {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "directory": path,
                            "config": config_json,
                            "modules": modules,
                        }))?
                    );
                } else {
                    println!("Config Dir: {}", path.display());
                    println!("Config File: {}", config_json.display());
                    println!("Modules Directory: {}", modules.display());
                }
                return Ok(());
            }
            Some(("show", _)) => {
                println!(
                    "{}",
                    serde_json::to_value(ConfigVerifier::from(config.clone()))
                        .and_then(|v| serde_json::to_string_pretty(&v))?
                );
                return Ok(());
            }
            _ => unreachable!(),
        },
        _ => (),
    }

    let emulator: Arc<dyn CTVEmulator> = if let Some(emcfg) = &config.active.emulator_nodes {
        if emcfg.enabled {
            emcfg.get_emulator()?.into()
        } else {
            Arc::new(CTVAvailable)
        }
    } else {
        Arc::new(CTVAvailable)
    };
    let plugin_map = config.active.plugin_map.map(|x| {
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
    use bitcoin::network::constants::Network;
    use rand::prelude::*;
    use std::str::FromStr;
    match matches.subcommand() {
        Some(("signer", sign_matches)) => match sign_matches.subcommand() {
            Some(("sign", args)) => {
                let buf = tokio::fs::read(args.value_of_os("input").unwrap()).await?;
                let xpriv = ExtendedPrivKey::decode(&buf)?;
                let psbt: PartiallySignedTransaction =
                    PartiallySignedTransaction::consensus_decode(
                        &base64::decode(&if let Some(psbt) = args.value_of("psbt") {
                            psbt.into()
                        } else {
                            let mut s = String::new();
                            tokio::io::stdin().read_to_string(&mut s).await?;
                            s
                        })?[..],
                    )?;
                let psbt = sign_psbt(&xpriv, psbt, &Secp256k1::new())?;
                let bytes = serialize(&psbt);
                if let Some(file_out) = args.value_of_os("out") {
                    std::fs::write(file_out, &base64::encode(bytes))?;
                } else {
                    println!("{}", base64::encode(bytes));
                }
            }
            Some(("new", args)) => {
                let mut entropy: [u8; 32] = rand::thread_rng().gen();
                let xpriv = ExtendedPrivKey::new_master(
                    Network::from_str(args.value_of("network").unwrap())?,
                    &entropy,
                )?;
                std::fs::write(args.value_of_os("out").unwrap(), &xpriv.encode())?;
                println!("{}", ExtendedPubKey::from_priv(&Secp256k1::new(), &xpriv));
            }
            Some(("show", args)) => {
                let buf = tokio::fs::read(args.value_of_os("input").unwrap()).await?;
                let xpriv = ExtendedPrivKey::decode(&buf)?;
                println!("{}", ExtendedPubKey::from_priv(&Secp256k1::new(), &xpriv));
            }
            _ => unreachable!(),
        },
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
                let pk_root = ExtendedPubKey::from_priv(&Secp256k1::new(), &root);
                let sync_mode = args.is_present("sync");
                let oracle = HDOracleEmulator::new(root, sync_mode);
                let interface = args.value_of("interface").unwrap();
                let server = oracle.bind(interface);
                let status = serde_json::json! {{
                    "interface": interface,
                    "pk": pk_root,
                    "sync": sync_mode,
                }};
                println!("{}", serde_json::to_string_pretty(&status).unwrap());
                server.await?;
            }
            _ => unreachable!(),
        },
        Some(("psbt", matches)) => match matches.subcommand() {
            Some(("finalize", args)) => {
                let psbt: PartiallySignedTransaction =
                    PartiallySignedTransaction::consensus_decode(
                        &base64::decode(&if let Some(psbt) = args.value_of("psbt") {
                            psbt.into()
                        } else {
                            let mut s = String::new();
                            tokio::io::stdin().read_to_string(&mut s).await?;
                            s
                        })?[..],
                    )?;
                let secp = Secp256k1::new();
                let js = psbt
                    .finalize(&secp)
                    .map(|tx| {
                        let hex = bitcoin::consensus::encode::serialize_hex(&tx.extract_tx());
                        serde_json::json!({
                            "completed": true,
                            "hex": hex
                        })
                    })
                    .unwrap_or_else(|(psbt, errors)| {
                        let errors: Vec<_> = errors.iter().map(|e| format!("{:?}", e)).collect();
                        let encoded_psbt = base64::encode(serialize(&psbt));
                        serde_json::json!(
                            {
                                 "completed": false,
                                 "psbt": encoded_psbt,
                                 "error": "Could not fully finalize psbt",
                                 "errors": errors
                            }
                        )
                    });
                println!("{}", serde_json::to_string_pretty(&js)?);
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
                )?;
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
                let use_base64 = args.is_present("base64_psbt");
                let outpoint: Option<bitcoin::OutPoint> = args
                    .value_of("outpoint")
                    .map(serde_json::from_str)
                    .transpose()?;
                let use_txn = args
                    .value_of("txn")
                    .map(|buf| base64::decode(buf.as_bytes()))
                    .transpose()?
                    .map(|b| PartiallySignedTransaction::consensus_decode(&b[..]))
                    .transpose()?;
                let client = rpc::Client::new(
                    config.active.api_node.url.clone(),
                    config.active.api_node.auth.clone(),
                )
                .await?;
                let j: Compiled = if let Some(json) = args.value_of("json") {
                    serde_json::from_str(json)?
                } else {
                    let mut s = String::new();
                    tokio::io::stdin().read_to_string(&mut s).await?;
                    serde_json::from_str(&s)?
                };

                let (tx, vout) = if use_mock {
                    let ctx = Context::new(
                        config.network,
                        j.amount_range.max(),
                        emulator.clone(),
                        "mock".try_into()?,
                        Arc::new(MapEffectDB::default()),
                    );
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

                        if let Some(psbt) = use_txn {
                            let script = a.script_pubkey();
                            if let Some(pos) = psbt
                                .unsigned_tx
                                .output
                                .iter()
                                .enumerate()
                                .find(|(_, o)| o.script_pubkey == script)
                                .map(|(i, _)| i)
                            {
                                (psbt.extract_tx(), pos as u32)
                            } else {
                                return Err(format!(
                                    "No Output found {:?} {:?}",
                                    psbt.unsigned_tx, a
                                )
                                .into());
                            }
                        } else {
                            let res = client
                                .wallet_create_funded_psbt(&[], &spends, None, None, None)
                                .await?;
                            let psbt = PartiallySignedTransaction::consensus_decode(
                                &base64::decode(&res.psbt)?[..],
                            )?;
                            let tx = psbt.extract_tx();
                            // if change pos is -1, then +1%len == 0. if it is 0, then 1. if 1, then 2 % len == 0.
                            let vout = ((res.change_position + 1) as usize) % tx.output.len();
                            (tx, vout as u32)
                        }
                    } else {
                        return Err("Must have a valid address".into());
                    }
                };
                let logger = Rc::new(TxIndexLogger::new());
                (*logger).add_tx(Arc::new(tx.clone()))?;

                let mut bound = j.bind_psbt(
                    OutPoint::new(tx.txid(), vout as u32),
                    BTreeMap::new(),
                    logger,
                    emulator.as_ref(),
                )?;

                if outpoint.is_none() {
                    let added_output_metadata = vec![OutputMeta::default(); tx.output.len()];
                    let output_metadata = vec![ObjectMetadata::default(); tx.output.len()];
                    let out = tx.input[0].previous_output;
                    let psbt = PartiallySignedTransaction::from_unsigned_tx(tx)?;
                    bound.program.insert(
                        SArc(Arc::new("funding".try_into()?)),
                        SapioStudioObject {
                            metadata: Default::default(),
                            out,
                            continue_apis: Default::default(),
                            txs: vec![LinkedPSBT {
                                psbt,
                                metadata: TemplateMetadata {
                                    label: Some("funding".into()),
                                    color: Some("pink".into()),
                                    extra: BTreeMap::new(),
                                    simp: Default::default(),
                                },
                                output_metadata,
                                added_output_metadata,
                            }
                            .into()],
                        },
                    );
                }
                if use_base64 {
                    println!("{}", serde_json::to_string_pretty(&bound)?);
                } else {
                    println!("{}", serde_json::to_string_pretty(&bound)?);
                }
            }
            Some(("create", args)) => {
                let sph = WasmPluginHandle::new_async(
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
                let create_args: CreateArgs<serde_json::Value> = serde_json::from_value(params)?;

                let v = sph.create(&PathFragment::Root.into(), &create_args)?;
                println!("{}", serde_json::to_string(&v)?);
            }
            Some(("api", args)) => {
                let sph = WasmPluginHandle::new_async(
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
            Some(("logo", args)) => {
                let sph = WasmPluginHandle::new_async(
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
                println!("{}", sph.get_logo()?);
            }
            Some(("info", args)) => {
                let sph = WasmPluginHandle::new_async(
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
                let sph = WasmPluginHandle::new_async(
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
