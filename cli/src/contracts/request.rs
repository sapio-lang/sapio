// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
use bitcoin::{psbt::Psbt, OutPoint};
use bitcoincore_rpc as rpc;
use bitcoincore_rpc::RpcApi;
use emulator_connect::CTVEmulator;
use sapio::{
    contract::{
        object::{LinkedPSBT, ObjectMetadata, Program, SapioStudioObject},
        Compiled,
    },
    template::{OutputMeta, TemplateMetadata},
    Context,
};
use sapio_base::{
    effects::{EffectPath, MapEffectDB, PathFragment},
    serialization_helpers::SArc,
    txindex::{TxIndex, TxIndexError, TxIndexLogger},
};
use sapio_wasm_plugin::{
    host::{plugin_handle::ModuleLocator, PluginHandle, WasmPluginHandle},
    CreateArgs, OrdinalsInfo, API,
};
use schemars::JsonSchema;
use serde::*;
use serde_json::Value;
use std::fmt::{Display, Formatter};
use std::{
    collections::{BTreeMap, HashMap},
    convert::TryInto,
    error::Error,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

use crate::{config::CovenantConfig, util::create_mock_output};

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Common {
    pub path: PathBuf,
    pub module_locator: Option<ModuleLocator>,
    pub net: bitcoin::Network,
    pub plugin_map: Option<BTreeMap<Vec<u8>, [u8; 32]>>,
}
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct List;
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ListReturn {
    items: BTreeMap<String, String>,
}
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct Call {
    pub params: serde_json::Value,
}
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CallReturn {
    result: Value,
}
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bind {
    pub covenant: CovenantConfig,
    pub client_url: String,
    #[serde(with = "crate::config::Auth")]
    #[schemars(with = "crate::config::Auth")]
    pub client_auth: rpc::Auth,
    pub use_mock: bool,
    pub outpoint: Option<OutPoint>,
    pub use_txn: Option<String>,
    pub compiled: Compiled,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ordinals_info: Option<OrdinalsInfo>,
}
pub type BindReturn = Program;
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct Api;
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ApiReturn {
    api: API<CreateArgs<Value>, Value>,
}
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct Logo;
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct LogoReturn {
    logo: String,
}
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct Info;
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct InfoReturn {
    name: String,
    description: String,
}
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct Load;
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct LoadReturn {
    key: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub enum Command {
    List(List),
    Call(Call),
    Bind(Bind),
    Api(Api),
    Logo(Logo),
    Info(Info),
    Load(Load),
}
#[derive(Serialize, Deserialize, JsonSchema)]
pub enum CommandReturn {
    List(ListReturn),
    Call(CallReturn),
    Bind(BindReturn),
    Api(ApiReturn),
    Logo(LogoReturn),
    Info(InfoReturn),
    Load(LoadReturn),
}

impl CommandReturn {
    /// The CLI writes composable payloads; Studio retains its protocol envelope.
    pub fn into_payload(self) -> ResultT<Value> {
        Ok(match self {
            Self::Call(value) => value.result,
            Self::Bind(value) => serde_json::to_value(value)?,
            Self::Api(value) => serde_json::to_value(value.api)?,
            Self::List(value) => serde_json::to_value(value.items)?,
            Self::Logo(value) => value.logo.into(),
            Self::Info(value) => serde_json::to_value(value)?,
            Self::Load(value) => serde_json::to_value(value)?,
        })
    }
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct Request {
    pub context: Common,
    pub command: Command,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct Response {
    pub result: Result<CommandReturn, RequestError>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug)]
pub struct RequestError(Value);

impl Display for RequestError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Value::String(message) => f.write_str(message),
            value => Display::fmt(value, f),
        }
    }
}

impl Error for RequestError {}

type ResultT<T> = Result<T, Box<dyn Error>>;
impl Request {
    pub async fn handle(self) -> Response {
        let v = self.handle_inner().await.map_err(|e| -> RequestError {
            e.downcast::<RequestError>()
                .map(|d| *d)
                .unwrap_or_else(|e| RequestError(e.to_string().into()))
        });
        Response { result: v }
    }
    pub async fn handle_inner(self) -> ResultT<CommandReturn> {
        let Request { context, command } = self;
        let Common {
            path,
            module_locator,
            net,
            plugin_map,
            ..
        } = context;
        match command {
            Command::List(_list) => {
                let plugins =
                    sapio_wasm_plugin::host::metadata::list(&path, context.net, plugin_map)?;
                let m = plugins
                    .into_iter()
                    .map(|p| (p.key, p.name))
                    .collect::<BTreeMap<_, _>>();
                Ok(CommandReturn::List(ListReturn { items: m }))
            }
            Command::Call(call) => {
                let params = call.params;
                let create_args: CreateArgs<serde_json::Value> = serde_json::from_value(params)?;
                if create_args.context.network != net {
                    return Err("module network does not match context.network".into());
                }
                let mut sph = WasmPluginHandle::<Value>::new_async(
                    &path,
                    module_locator.ok_or("Expected to have exactly one of key or file")?,
                    net,
                    plugin_map,
                )
                .await?;
                let v = sph.call(&PathFragment::Root.into(), &create_args)?;
                Ok(CommandReturn::Call(CallReturn { result: v }))
            }
            Command::Bind(bind) => {
                let emulator = bind.covenant.get_emulator().await?;
                Ok(CommandReturn::Bind(bind.call(net, emulator).await?))
            }
            Command::Api(_api) => {
                let metadata = sapio_wasm_plugin::host::metadata::get_async(
                    &path,
                    module_locator.ok_or("Expected to have exactly one of key or file")?,
                    net,
                    plugin_map,
                )
                .await?;
                Ok(CommandReturn::Api(ApiReturn { api: metadata.api }))
            }
            Command::Logo(_logo) => {
                let metadata = sapio_wasm_plugin::host::metadata::get_async(
                    &path,
                    module_locator.ok_or("Expected to have exactly one of key or file")?,
                    net,
                    plugin_map,
                )
                .await?;
                Ok(CommandReturn::Logo(LogoReturn {
                    logo: metadata.logo,
                }))
            }
            Command::Info(_info) => {
                let metadata = sapio_wasm_plugin::host::metadata::get_async(
                    &path,
                    module_locator.ok_or("Expected to have exactly one of key or file")?,
                    net,
                    plugin_map,
                )
                .await?;
                let api = metadata.api;
                Ok(CommandReturn::Info(InfoReturn {
                    name: metadata.name,
                    description: api
                        .input()
                        .as_value()
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                }))
            }
            Command::Load(_load) => {
                let metadata = sapio_wasm_plugin::host::metadata::get_async(
                    &path,
                    module_locator.ok_or("Expected to have exactly one of key or file")?,
                    net,
                    plugin_map,
                )
                .await?;
                Ok(CommandReturn::Load(LoadReturn { key: metadata.key }))
            }
        }
    }
}

impl Bind {
    async fn call(
        self,
        net: bitcoin::Network,
        emulator: Arc<dyn CTVEmulator>,
    ) -> Result<BindReturn, Box<dyn Error>> {
        let Bind {
            covenant,
            client_url,
            client_auth,
            use_mock,
            use_txn,
            compiled,
            outpoint,
            ordinals_info,
        } = self;
        if u8::from(use_mock) + u8::from(outpoint.is_some()) + u8::from(use_txn.is_some()) > 1 {
            return Err(Box::new(RequestError(
                "binding funding sources are mutually exclusive".into(),
            )));
        }
        compiled.validate_for_emulator(emulator.as_ref())?;
        if !covenant.allows_native_ctv() && compiled.requires_native_ctv() {
            return Err(Box::new(RequestError(
                "Artifact requires native CTV; signer_emulation does not establish native CTV enforcement. Select a mode explicitly including native_ctv_research only when the target chain is assumed to enforce it."
                    .into(),
            )));
        }
        let use_txn = use_txn
            .map(|buf| base64::decode(buf.as_bytes()))
            .transpose()?
            .map(|b| Psbt::deserialize(&b))
            .transpose()?;
        if let Some(psbt) = &use_txn {
            sapio_psbt::validate_psbt(psbt)?;
        }
        let (tx, vout, funding_psbt) = if use_mock {
            let ctx = Context::new(
                net,
                compiled.required_input_amount,
                compiled.covenant_requirements.lowering.clone(),
                "mock".try_into()?,
                Arc::new(MapEffectDB::default()),
                ordinals_info,
            );
            let mut tx = ctx
                .template()
                .add_output(compiled.required_input_amount, &compiled, None)?
                .get_tx();
            tx.input[0].previous_output = create_mock_output();
            let psbt = if outpoint.is_none() {
                Some(Psbt::from_unsigned_tx(tx.clone())?)
            } else {
                None
            };
            (tx, 0, psbt)
        } else if let Some(outpoint) = outpoint {
            let client = rpc::Client::new(&client_url, client_auth)?;
            let res = tokio::task::spawn_blocking(move || {
                client.get_raw_transaction(&outpoint.txid, None)
            })
            .await??;
            validate_funding_outpoint(&res, outpoint)?;
            (res, outpoint.vout, None)
        } else {
            let mut spends = HashMap::new();
            let script = bitcoin::ScriptBuf::from(&compiled.address);
            if let Ok(a) = bitcoin::Address::from_script(&script, net) {
                spends.insert(format!("{}", a), compiled.required_input_amount);

                let psbt = if let Some(psbt) = use_txn {
                    psbt
                } else {
                    let client = rpc::Client::new(&client_url, client_auth)?;
                    let res = tokio::task::spawn_blocking(move || {
                        client.wallet_create_funded_psbt(&[], &spends, None, None, None)
                    })
                    .await??;
                    Psbt::deserialize(&base64::decode(&res.psbt)?)?
                };
                let vout = funding_output(&psbt, &script)?;
                // Final scriptSigs can change the TXID. Bind the extracted
                // transaction while retaining the complete PSBT for signing.
                (psbt.clone().extract_tx()?, vout, Some(psbt))
            } else {
                return Err(Err(RequestError("Must have a valid address".into()))?);
            }
        };
        let logger = Rc::new(TxIndexLogger::new());
        (*logger).add_tx(Arc::new(tx.clone()))?;
        let mut bound = compiled.bind_psbt(
            OutPoint::new(tx.compute_txid(), vout),
            BTreeMap::new(),
            logger,
            emulator.as_ref(),
        )?;
        if let Some(psbt) = funding_psbt {
            let added_output_metadata = vec![OutputMeta::default(); tx.output.len()];
            let output_metadata = vec![ObjectMetadata::default(); tx.output.len()];
            let out = tx.input[0].previous_output;
            bound.program.insert(
                SArc(EffectPath::push(
                    Some(compiled.root_path.0.clone()),
                    PathFragment::Funding,
                )),
                SapioStudioObject {
                    source_path: None,
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
        Ok(bound)
    }
}

fn funding_output(psbt: &Psbt, script: &bitcoin::ScriptBuf) -> ResultT<u32> {
    sapio_psbt::validate_psbt(psbt)?;
    let index = psbt
        .unsigned_tx
        .output
        .iter()
        .position(|output| output.script_pubkey == *script)
        .ok_or_else(|| {
            RequestError("Funding transaction has no output paying the contract".into())
        })?;
    Ok(index.try_into()?)
}

fn validate_funding_outpoint(
    tx: &bitcoin::Transaction,
    outpoint: OutPoint,
) -> Result<(), TxIndexError> {
    let actual = tx.compute_txid();
    if actual != outpoint.txid {
        return Err(TxIndexError::TxidMismatch {
            expected: outpoint.txid,
            actual,
        });
    }
    if outpoint.vout as usize >= tx.output.len() {
        return Err(TxIndexError::IndexTooHigh(outpoint.vout));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
