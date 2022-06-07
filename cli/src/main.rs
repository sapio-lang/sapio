use crate::contracts::Response;
// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
#[deny(missing_docs)]
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
use bitcoin::consensus::serialize;
use bitcoin::consensus::Decodable;
use bitcoin::network::constants::Network;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::util::bip32::ExtendedPrivKey;
use bitcoin::util::bip32::ExtendedPubKey;
use bitcoin::util::psbt::PartiallySignedTransaction;
use clap::clap_app;
use clap::ArgMatches;
use config::*;
use emulator_connect::servers::hd::HDOracleEmulator;
use emulator_connect::CTVAvailable;
use emulator_connect::CTVEmulator;
use miniscript::psbt::PsbtExt;
use rand::prelude::*;
use sapio::contract::Compiled;
use sapio_base::util::CTVHash;
use schemars::schema_for;
use serde_json::Deserializer;
use std::error::Error;
use std::ffi::OsString;
use std::str::FromStr;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::sync::oneshot;
use util::*;
pub mod config;
mod contracts;
mod util;

async fn config(matches: &ArgMatches) -> Result<Config, Box<dyn Error>> {
    Config::setup(&matches, "org", "judica", "sapio-cli").await
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
                let config = config(&matches).await?;
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
                let entropy: [u8; 32] = rand::thread_rng().gen();
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
        Some(("emulator", sign_matches)) => {
            let config = config(&matches).await?;
            let emulator: Arc<dyn CTVEmulator> = if let Some(emcfg) = &config.active.emulator_nodes
            {
                if emcfg.enabled {
                    emcfg.get_emulator()?.into()
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
                    if let Some(_) = Arc::get_mut(&mut emulator) {
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
        Some(("studio", matches)) => match matches.subcommand() {
            Some(("server", args)) => {
                let (server, send_server, shutdown_server) = Server::new();
                server.run();
                if args.is_present("stdin") {
                    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
                    // TODO: This should be using Tokio, somehow...  because we
                    // use the std::io::stdin blocking interface, we wrap it in
                    // a spawn_blocking so that we don't ruin the runtime.  we
                    // use a channel to send the values back & do most of the
                    // processing there because we want to use ? error handling.
                    let stream = tokio::task::spawn_blocking(move || {
                        let stream =
                            Deserializer::from_reader(std::io::stdin()).into_iter::<Request>();
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
            let config = config(&matches).await?;
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
                    .map(|(x, y)| (x.into_bytes().into(), y.into()))
                    .collect()
            });
            let context = |args: &clap::ArgMatches| Common {
                path: module_path(args),
                emulator: emulator_args,
                key: args.value_of("key").map(ToString::to_string),
                file: args.value_of_os("file").map(OsString::from),
                net: network,
                plugin_map,
            };
            let (server, send_server, shutdown_server) = Server::new();

            let msg = match matches.subcommand() {
                Some(("bind", args)) => {
                    let use_mock = args.is_present("mock");
                    let use_base64 = args.is_present("base64_psbt");
                    let outpoint: Option<bitcoin::OutPoint> = args
                        .value_of("outpoint")
                        .map(serde_json::from_str)
                        .transpose()?;
                    let use_txn = args.value_of("txn").map(String::from);
                    let client_url = config.active.api_node.url.clone();
                    let client_auth = config.active.api_node.auth.clone();
                    let compiled: Compiled = if let Some(json) = args.value_of("json") {
                        serde_json::from_str(json)?
                    } else {
                        let mut s = String::new();
                        tokio::io::stdin().read_to_string(&mut s).await?;
                        serde_json::from_str(&s)?
                    };
                    Request {
                        context: context(&args),
                        command: Command::Bind(Bind {
                            client_url,
                            client_auth,
                            use_base64,
                            use_mock,
                            outpoint,
                            use_txn,
                            compiled,
                        }),
                    }
                }
                Some(("list", args)) => Request {
                    context: context(&args),
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
                        context: context(&args),
                        command: Command::Call(Call { params }),
                    }
                }
                Some(("api", args)) => Request {
                    context: context(&args),
                    command: Command::Api(Api),
                },
                Some(("logo", args)) => Request {
                    context: context(&args),
                    command: Command::Logo(Logo),
                },
                Some(("info", args)) => Request {
                    context: context(&args),
                    command: Command::Info(Info),
                },
                Some(("load", args)) => Request {
                    context: context(&args),
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
