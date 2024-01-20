// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![deny(missing_docs)]

//! command line interface for manipulating sapio contracts and other related tasks

use crate::contracts::server::Server;
use crate::contracts::Api;
use crate::contracts::Bind;
use crate::contracts::Call;
use crate::contracts::Command;
use crate::contracts::Common;
use crate::contracts::Info;
use crate::contracts::List;
use crate::contracts::Load;
use crate::contracts::Logo;
use crate::contracts::Request;
use crate::contracts::Response;
use bitcoin::consensus::serialize;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::util::bip32::ExtendedPrivKey;
use bitcoin::util::bip32::ExtendedPubKey;
use bitcoin::Network;
use clap::clap_app;
use clap::ArgMatches;
use config::*;
use emulator_connect::servers::hd::HDOracleEmulator;
use emulator_connect::CTVAvailable;
use emulator_connect::CTVEmulator;
use sapio::contract::Compiled;
use sapio_base::util::CTVHash;
use sapio_wasm_plugin::host::plugin_handle::ModuleLocator;
use schemars::schema_for;
use serde_json::Deserializer;
use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::sync::oneshot;
use util::*;
pub mod config;
mod contracts;
mod util;

async fn config(custom_config: Option<&str>) -> Result<Config, Box<dyn Error>> {
    Config::setup(custom_config, "org", "judica", "sapio-cli").await
}

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
    (@subcommand studio =>
     (@setting SubcommandRequiredElseHelp)
     (about: "commands for sapio studio integration")
     (@subcommand server =>
      (about: "run a studio server")
      (@group from +required =>
        (@arg stdin: --stdin  "Run in Synchronous mode")
        (@arg interface: --interface +takes_value "The Interface to Bind")
      )
     )
     (@subcommand schemas =>
      (about: "print input and output schemas")
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
       (@arg workspace: -w --workspace +takes_value "Where to search for the cache / copy the contract file")
       (@group from +required =>
        (@arg file: -f --file +takes_value {check_file} "Which Contract to Create, given a WASM Plugin file")
        (@arg key:  -k --key +takes_value "Which Contract to Create, given a WASM Hash")
       )
       (@arg json: "JSON of args")
      )
      (@subcommand load =>
       (about: "Load a wasm contract module, returns the hex sha3 hash key")
       (@arg workspace: -w --workspace +takes_value "Where to copy the contract file")
       (@arg file: -f --file +required +takes_value {check_file} "Which Contract to Create, given a WASM Plugin file")
      )
      (@subcommand api =>
       (about: "Machine Readable API for a plugin, pipe into jq for pretty formatting.")
       (@arg workspace: -w --workspace +takes_value "Where to search for the cache / copy the contract file")
       (@group from +required =>
        (@arg file: -f --file +takes_value {check_file} "Which Contract to Create, given a WASM Plugin file")
        (@arg key:  -k --key +takes_value "Which Contract to Create, given a WASM Hash")
       )
      )
      (@subcommand logo =>
       (about: "base64 encoded png image")
       (@arg workspace: -w --workspace +takes_value "Where to search for the cache / copy the contract file")
       (@group from +required =>
        (@arg file: -f --file +takes_value {check_file} "Which Contract to Create, given a WASM Plugin file")
        (@arg key:  -k --key +takes_value "Which Contract to Create, given a WASM Hash")
       )
      )
      (@subcommand info =>
       (about: "View human readable basic information for a plugin")
       (@arg workspace: -w --workspace +takes_value "Where to search for the cache / copy the contract file")
       (@group from +required =>
        (@arg file: -f --file +takes_value {check_file} "Which Contract to Create, given a WASM Plugin file")
        (@arg key:  -k --key +takes_value "Which Contract to Create, given a WASM Hash")
       )
      )
      (@subcommand list =>
       (about: "list available contracts")
       (@arg workspace: -w --workspace +takes_value "Where to search for.")
      )
      )
      );
    let matches = app.get_matches();
    let custom_config = matches.value_of("config");
    match matches.subcommand() {
        Some(("configure", config_matches)) => match config_matches.subcommand() {
            Some(("wizard", args)) => {
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
                let config = config(custom_config).await?;
                println!(
                    "{}",
                    serde_json::to_value(ConfigVerifier::from(config))
                        .and_then(|v| serde_json::to_string_pretty(&v))?
                );
                return Ok(());
            }
            _ => unreachable!(),
        },
        Some(("signer", sign_matches)) => match sign_matches.subcommand() {
            Some(("sign", args)) => {
                let input = args.value_of_os("input").unwrap();
                let psbt_str = args.value_of("psbt");
                let output = args.value_of_os("out");

                let buf = tokio::fs::read(input).await?;
                let xpriv = sapio_psbt::SigningKey::read_key_from_buf(&buf[..])?;
                let psbt = get_psbt_from(psbt_str).await?;
                let hash_ty = bitcoin::util::sighash::SchnorrSighashType::All;
                let bytes = xpriv.sign(psbt, hash_ty)?;

                if let Some(file_out) = output {
                    std::fs::write(file_out, &base64::encode(bytes))?;
                } else {
                    println!("{}", base64::encode(bytes));
                }
            }
            Some(("new", args)) => {
                let network = args.value_of("network").unwrap();
                let network = Network::from_str(network)?;
                let out = args.value_of_os("out").unwrap();
                let xpriv = sapio_psbt::SigningKey::new_key(network)?;
                let pubkey = xpriv.pubkey(&Secp256k1::new());
                tokio::fs::write(out, &xpriv.0[0].encode()).await?;
                println!("{}", pubkey[0]);
            }
            Some(("show", args)) => {
                let input = args.value_of_os("input").unwrap();
                let buf = tokio::fs::read(input).await?;
                let xpriv = sapio_psbt::SigningKey::read_key_from_buf(&buf[..])?;
                let pubkey = xpriv.pubkey(&Secp256k1::new());
                println!("{}", pubkey[0]);
            }
            _ => unreachable!(),
        },
        Some(("emulator", sign_matches)) => {
            let config = config(custom_config).await?;
            let emulator: Arc<dyn CTVEmulator> = if let Some(emcfg) = &config.active.emulator_nodes
            {
                if emcfg.enabled {
                    emcfg.get_emulator()?
                } else {
                    Arc::new(CTVAvailable)
                }
            } else {
                Arc::new(CTVAvailable)
            };
            // TODO: is this still required to drop the emulator from a unique thread?
            {
                let mut emulator = emulator.clone();
                // Drop Emulator from own thread...
                std::thread::spawn(move || loop {
                    if Arc::get_mut(&mut emulator).is_some() {
                        break;
                    }
                });
            }
            match sign_matches.subcommand() {
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
            }
        }
        Some(("psbt", matches)) => match matches.subcommand() {
            Some(("finalize", args)) => {
                let psbt_str = args.value_of("psbt");

                let psbt = get_psbt_from(psbt_str).await?;
                let js = sapio_psbt::external_api::finalize_psbt_format_api(psbt);
                println!("{}", serde_json::to_string_pretty(&js)?);
            }
            _ => unreachable!(),
        },
        Some(("studio", matches)) => match matches.subcommand() {
            Some(("server", args)) => {
                let from_stdin = args.is_present("stdin");
                if from_stdin {
                    run_server_stdin().await?;
                } else {
                    args.value_of("interface");
                }
            }
            Some(("schemas", _args)) => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&schema_for!((Request, Response)))?,
                );
            }
            _ => unreachable!(),
        },
        Some(("contract", matches)) => {
            let config = config(custom_config).await?;
            let module_path = |args: &clap::ArgMatches| {
                let mut p = args
                    .value_of("workspace")
                    .map(Into::into)
                    .unwrap_or_else(|| util::get_data_dir("org", "judica", "sapio-cli"));
                p.push("modules");
                p
            };
            let network = config.network;
            let emulator_args = config.active.emulator_nodes;
            let plugin_map = config.active.plugin_map.map(|x| {
                x.into_iter()
                    .map(|(x, y)| (x.into_bytes(), y.into()))
                    .collect()
            });
            let context = |args: &clap::ArgMatches| -> Result<Common, &'static str> {
                let module_locator = args
                    .value_of("file")
                    .map(String::from)
                    .map(ModuleLocator::FileName)
                    .xor(
                        args.value_of("key")
                            .map(ToString::to_string)
                            .map(ModuleLocator::Key),
                    );
                Ok(Common {
                    path: module_path(args),
                    emulator: emulator_args,
                    module_locator,
                    net: network,
                    plugin_map,
                })
            };
            let (server, send_server, shutdown_server) = Server::new();

            let msg = match matches.subcommand() {
                Some(("bind", args)) => {
                    let client_url = config.active.api_node.url.clone();
                    let client_auth = config.active.api_node.auth.clone();
                    Request {
                        context: context(args)?,
                        command: bind_command(args, client_url, client_auth).await?,
                    }
                }
                Some(("list", args)) => Request {
                    context: context(args)?,
                    command: Command::List(List),
                },
                Some(("create", args)) => {
                    let json = args.value_of("json").map(|x| x.to_string());
                    let params = if let Some(params) = json {
                        serde_json::from_str(&params)?
                    } else {
                        let mut s = String::new();
                        tokio::io::stdin().read_to_string(&mut s).await?;
                        serde_json::from_str(&s)?
                    };
                    Request {
                        context: context(args)?,
                        command: Command::Call(Call { params }),
                    }
                }
                Some(("api", args)) => Request {
                    context: context(args)?,
                    command: Command::Api(Api),
                },
                Some(("logo", args)) => Request {
                    context: context(args)?,
                    command: Command::Logo(Logo),
                },
                Some(("info", args)) => Request {
                    context: context(args)?,
                    command: Command::Info(Info),
                },
                Some(("load", args)) => Request {
                    context: context(args)?,
                    command: Command::Load(Load),
                },
                _ => unreachable!(),
            };
            server.run();
            let (tx, rx) = oneshot::channel();
            send_server.send((msg, tx)).map_err(|_e| "Failed to Send")?;
            println!("{}", serde_json::to_string_pretty(&rx.await?)?);
            shutdown_server.send(())?;
        }
        _ => unreachable!(),
    };

    Ok(())
}

async fn run_server_stdin() -> Result<(), Box<dyn Error>> {
    let (server, send_server, shutdown_server) = Server::new();
    server.run();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let stream = tokio::task::spawn_blocking(move || {
        let stream = Deserializer::from_reader(std::io::stdin()).into_iter::<Request>();
        for json in stream {
            // if a bad json is read, break.
            if tx.send(json).is_err() {
                break;
            }
        }
    });
    while let Some(json) = rx.recv().await {
        let (b_tx, b_rx) = oneshot::channel();
        send_server
            .send((json?, b_tx))
            .map_err(|_e| "Failed to Send")?;

        println!("{}", serde_json::to_string_pretty(&b_rx.await?)?);
    }
    shutdown_server.send(())?;
    stream.await?;
    Ok(())
}

async fn bind_command(
    args: &ArgMatches,
    client_url: String,
    client_auth: bitcoincore_rpc_async::Auth,
) -> Result<Command, Box<dyn Error>> {
    let use_mock = args.is_present("mock");
    let ordinals_info = None;
    let use_base64 = args.is_present("base64_psbt");
    let outpoint: Option<bitcoin::OutPoint> = args
        .value_of("outpoint")
        .map(serde_json::from_str)
        .transpose()?;
    let use_txn = args.value_of("txn").map(String::from);
    let compiled: Compiled = if let Some(json) = args.value_of("json") {
        serde_json::from_str(json)?
    } else {
        let mut s = String::new();
        tokio::io::stdin().read_to_string(&mut s).await?;
        serde_json::from_str(&s)?
    };
    Ok(Command::Bind(Bind {
        client_url,
        client_auth,
        use_base64,
        use_mock,
        outpoint,
        use_txn,
        compiled,
        ordinals_info
    }))
}
