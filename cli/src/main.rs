// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![deny(missing_docs)]

//! Compile, inspect and spend Sapio contracts from explicit files and inputs.

use args::{Cli, Command};
use bitcoin::bip32::{Xpriv, Xpub};
use bitcoin::secp256k1::Secp256k1;
use clap::Parser;
use config::{Config, ConfigVerifier};
use contracts::{server::Server, Common, Request, Response};
use emulator_connect::servers::hd::HDOracleEmulator;
use emulator_connect::CTVEmulator;
use sapio_wasm_plugin::host::plugin_handle::ModuleLocator;
use sapio_wasm_plugin::CreateArgs;
use schemars::generate::SchemaSettings;
use serde_json::{Deserializer, Value};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use tokio::sync::oneshot;
use util::{
    read_input, read_json, read_psbt, write_json, write_new, write_output, write_psbt, Result,
};

mod args;
pub mod config;
mod contracts;
mod explain;
mod spend;
mod util;

async fn config(custom_config: Option<&Path>) -> Result<Config> {
    Config::setup(custom_config, "org", "judica", "sapio-cli").await
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    let custom_config = cli.config.as_deref();
    match cli.command {
        Command::Configure { command } => configure(command, custom_config).await,
        Command::Signer { command } => match command {
            args::Signer::Sign { key, psbt, output } => {
                let key = sapio_psbt::SigningKey::read_key_from_buf(&read_input(Some(&key))?)?;
                let signed = key.sign(
                    read_psbt(psbt.as_deref())?,
                    bitcoin::sighash::TapSighashType::All,
                )?;
                write_output(output.output.as_deref(), base64::encode(signed).as_bytes())
            }
            args::Signer::New { network, output } => {
                let key = sapio_psbt::SigningKey::new_key(network)?;
                let public = key.pubkey(&Secp256k1::new());
                write_new(&output, &key.0[0].encode())?;
                write_output(None, public[0].to_string().as_bytes())
            }
            args::Signer::Show { input } => {
                let key = sapio_psbt::SigningKey::read_key_from_buf(&read_input(Some(&input))?)?;
                write_output(
                    None,
                    key.pubkey(&Secp256k1::new())[0].to_string().as_bytes(),
                )
            }
        },
        Command::Emulator { command } => match command {
            args::Emulator::Sign { psbt, output } => {
                let emulator = configured_emulator(custom_config).await?;
                write_psbt(
                    output.output.as_deref(),
                    &emulator.sign(read_psbt(psbt.as_deref())?)?,
                )
            }
            args::Emulator::GetKey { psbt } => {
                let emulator = configured_emulator(custom_config).await?;
                let key = emulator.get_signer_for(util::ctv_hash(&read_psbt(psbt.as_deref())?)?)?;
                write_output(None, key.to_string().as_bytes())
            }
            args::Emulator::Show { psbt } => write_json(None, &read_psbt(psbt.as_deref())?),
            args::Emulator::Server {
                seed,
                interface,
                request_timeout_secs,
                max_connections,
            } => {
                let config = config(custom_config).await?;
                let root = Xpriv::new_master(config.network, &read_input(Some(&seed))?)?;
                let public = Xpub::from_priv(&Secp256k1::new(), &root);
                let oracle = HDOracleEmulator::new(root).with_limits(
                    std::time::Duration::from_secs(request_timeout_secs),
                    max_connections,
                )?;
                let listener = tokio::net::TcpListener::bind(interface).await?;
                // A single JSON line is the readiness message consumed by clients.
                write_output(
                    None,
                    &serde_json::to_vec(&serde_json::json!({
                        "interface": listener.local_addr()?,
                        "pk": public,
                        "request_timeout_secs": request_timeout_secs,
                        "max_connections": max_connections,
                    }))?,
                )?;
                oracle.serve(listener).await?;
                Ok(())
            }
        },
        Command::Psbt { command } => match command {
            args::Psbt::Finalize { psbt, output } => write_json(
                output.output.as_deref(),
                &sapio_psbt::external_api::finalize_psbt_format_api(read_psbt(psbt.as_deref())?)?,
            ),
        },
        Command::Studio { command } => match command {
            args::Studio::Server { stdin: _ } => run_server_stdin().await,
            args::Studio::Schemas => write_json(
                None,
                &SchemaSettings::draft07()
                    .into_generator()
                    .into_root_schema_for::<(Request, Response)>(),
            ),
        },
        Command::Contract { command } => contract(command, custom_config).await,
    }
}

async fn configure(command: args::Configure, custom_config: Option<&Path>) -> Result<()> {
    match command {
        args::Configure::Files { json } => {
            let project = util::project_dirs()?;
            let path = config_path(custom_config)?;
            let modules = util::module_path(None)?;
            if json {
                write_json(
                    None,
                    &serde_json::json!({
                        "directory": project.config_dir(), "config": path, "modules": modules,
                    }),
                )
            } else {
                write_output(
                    None,
                    format!(
                        "Config file: {}\nModules directory: {}",
                        path.display(),
                        modules.display()
                    )
                    .as_bytes(),
                )
            }
        }
        args::Configure::Show => write_json(None, &config::redacted(config(custom_config).await?)?),
        args::Configure::Wizard { write } => {
            let settings = ConfigVerifier::wizard().await?;
            if write {
                let path = config_path(custom_config)?;
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    std::fs::create_dir_all(parent)?;
                }
                write_new(&path, &serde_json::to_vec_pretty(&settings)?)?;
                eprintln!("Created configuration '{}'.", path.display());
                Ok(())
            } else {
                write_json(None, &settings)
            }
        }
    }
}

fn config_path(custom: Option<&Path>) -> Result<PathBuf> {
    match custom {
        Some(path) => Ok(path.into()),
        None => Ok(util::project_dirs()?.config_dir().join("config.json")),
    }
}

fn module_context(
    options: args::ModuleOptions,
    source: Option<args::ModuleSource>,
) -> Result<Common> {
    let module_locator = match source {
        Some(args::ModuleSource {
            file: Some(file), ..
        }) => Some(ModuleLocator::FileName(
            file.into_os_string()
                .into_string()
                .map_err(|_| "WASM module path must be valid UTF-8")?,
        )),
        Some(args::ModuleSource { key: Some(key), .. }) => Some(ModuleLocator::Key(key)),
        _ => None,
    };
    let plugin_map = options
        .plugin_map
        .as_deref()
        .map(|path| {
            read_json::<std::collections::BTreeMap<String, config::WasmerCacheHash>>(Some(path))
        })
        .transpose()?;
    Ok(Common {
        path: util::module_path(options.workspace)?,
        module_locator,
        net: options.network.unwrap_or(bitcoin::Network::Regtest),
        plugin_map: plugin_map.map(|entries| {
            entries
                .into_iter()
                .map(|(alias, hash)| (alias.into_bytes(), hash.into()))
                .collect()
        }),
    })
}

async fn contract(command: args::Contract, custom_config: Option<&Path>) -> Result<()> {
    let (request, output) = match command {
        args::Contract::Explain(args) => return explain::run(args),
        args::Contract::Spend { command } => return spend::run(command),
        args::Contract::Create { mut module, args } => {
            let parameters: CreateArgs<Value> = read_json(args.as_deref())?;
            if module
                .options
                .network
                .is_some_and(|network| network != parameters.context.network)
            {
                return Err(
                    "--network does not match the required context.network in create arguments"
                        .into(),
                );
            }
            module.options.network = Some(parameters.context.network);
            (
                Request {
                    context: module_context(module.options, Some(module.source))?,
                    command: contracts::Command::Call(contracts::Call {
                        params: serde_json::to_value(parameters)?,
                    }),
                },
                module.output,
            )
        }
        args::Contract::Load {
            options,
            file,
            output,
        } => (
            Request {
                context: module_context(
                    options,
                    Some(args::ModuleSource {
                        file: Some(file),
                        key: None,
                    }),
                )?,
                command: contracts::Command::Load(contracts::Load),
            },
            output,
        ),
        args::Contract::Api(module) => (
            Request {
                context: module_context(module.options, Some(module.source))?,
                command: contracts::Command::Api(contracts::Api),
            },
            module.output,
        ),
        args::Contract::Info(module) => (
            Request {
                context: module_context(module.options, Some(module.source))?,
                command: contracts::Command::Info(contracts::Info),
            },
            module.output,
        ),
        args::Contract::Logo(module) => (
            Request {
                context: module_context(module.options, Some(module.source))?,
                command: contracts::Command::Logo(contracts::Logo),
            },
            module.output,
        ),
        args::Contract::List { options, output } => (
            Request {
                context: module_context(options, None)?,
                command: contracts::Command::List(contracts::List),
            },
            output,
        ),
        args::Contract::Bind {
            artifact,
            funding,
            output,
        } => {
            let compiled = read_json(artifact.as_deref())?;
            let config = config(custom_config).await?;
            let funding_psbt = funding
                .funding_psbt
                .as_deref()
                .map(|path| read_psbt(Some(path)))
                .transpose()?;
            (
                Request {
                    context: Common {
                        path: PathBuf::new(),
                        module_locator: None,
                        net: config.network,
                        plugin_map: None,
                    },
                    command: contracts::Command::Bind(contracts::Bind {
                        covenant: config.active.covenant,
                        client_url: config.active.api_node.url,
                        client_auth: config.active.api_node.auth,
                        use_mock: funding.mock,
                        outpoint: funding.outpoint,
                        use_txn: funding_psbt.map(|psbt| base64::encode(psbt.serialize())),
                        compiled,
                        ordinals_info: None,
                    }),
                },
                output,
            )
        }
    };
    // Direct CLI calls propagate operation failures. Only Studio uses Response's envelope.
    let payload = request.handle_inner().await?.into_payload()?;
    write_json(output.output.as_deref(), &payload)
}

async fn configured_emulator(custom_config: Option<&Path>) -> Result<Arc<dyn CTVEmulator>> {
    config(custom_config)
        .await?
        .active
        .covenant
        .get_emulator()
        .await
}

async fn run_server_stdin() -> Result<()> {
    let (server, send_server, shutdown_server) = Server::new();
    server.run();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let stream = tokio::task::spawn_blocking(move || {
        for json in Deserializer::from_reader(std::io::stdin()).into_iter::<Request>() {
            if tx.send(json).is_err() {
                break;
            }
        }
    });
    while let Some(json) = rx.recv().await {
        let (tx, rx) = oneshot::channel();
        send_server
            .send((json?, tx))
            .map_err(|_| "failed to send Studio request")?;
        write_json(None, &rx.await?)?;
    }
    shutdown_server.send(())?;
    stream.await?;
    Ok(())
}
