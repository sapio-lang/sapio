// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
use bitcoin::{consensus::deserialize, psbt::PartiallySignedTransaction, OutPoint};
use bitcoincore_rpc_async as rpc;
use bitcoincore_rpc_async::RpcApi;
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
use std::fmt::{Display, Formatter, Write};
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
    pub covenant: CovenantConfig,
    pub module_locator: Option<ModuleLocator>,
    #[schemars(with = "String")]
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
pub struct Bind {
    pub client_url: String,
    #[serde(with = "crate::config::Auth")]
    pub client_auth: rpc::Auth,
    pub use_base64: bool,
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
        write!(f, "{:?}", self)
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
        // create the future to get the sph,
        // but do not await it since not all calls will use it.
        let Request { context, command } = self;
        let Common {
            path,
            covenant,
            module_locator,
            net,
            plugin_map,
            ..
        } = context;
        let default_sph = || -> Result<_, &'static str> {
            Ok(WasmPluginHandle::<Value>::new_async(
                &path,
                module_locator.ok_or("Expected to have exactly one of key or file")?,
                net,
                plugin_map.clone(),
            ))
        };
        match command {
            Command::List(_list) => {
                let mut plugins =
                    WasmPluginHandle::<Value>::load_all_keys(&path, context.net, plugin_map)?;
                let m = plugins
                    .iter_mut()
                    .map(|p| p.get_name().map(|name| (p.id().to_string(), name)))
                    .collect::<Result<BTreeMap<_, _>, _>>()?;
                Ok(CommandReturn::List(ListReturn { items: m }))
            }
            Command::Call(call) => {
                let params = call.params;
                let mut sph = default_sph()?.await?;

                let create_args: CreateArgs<serde_json::Value> = serde_json::from_value(params)?;
                let v = sph.call(&PathFragment::Root.into(), &create_args)?;
                Ok(CommandReturn::Call(CallReturn { result: v }))
            }
            Command::Bind(bind) => {
                let emulator = covenant.get_emulator().await?;
                Ok(CommandReturn::Bind(
                    bind.call(net, emulator, &covenant).await?,
                ))
            }
            Command::Api(_api) => {
                let mut sph = default_sph()?.await?;
                Ok(CommandReturn::Api(ApiReturn {
                    api: sph.get_api()?,
                }))
            }
            Command::Logo(_logo) => {
                let mut sph = default_sph()?.await?;
                Ok(CommandReturn::Logo(LogoReturn {
                    logo: sph.get_logo()?,
                }))
            }
            Command::Info(_info) => {
                let mut sph = default_sph()?.await?;
                let api = sph.get_api()?;
                Ok(CommandReturn::Info(InfoReturn {
                    name: sph.get_name()?,
                    description: api
                        .input()
                        .schema
                        .metadata
                        .as_ref()
                        .and_then(|m| m.description.as_ref())
                        .unwrap()
                        .clone(),
                }))
            }
            Command::Load(_load) => {
                let sph = default_sph()?.await?;
                Ok(CommandReturn::Load(LoadReturn {
                    key: sph.id().to_string(),
                }))
            }
        }
    }
}

impl Bind {
    async fn call(
        self,
        net: bitcoin::Network,
        emulator: Arc<dyn CTVEmulator>,
        covenant: &CovenantConfig,
    ) -> Result<BindReturn, Box<dyn Error>> {
        let Bind {
            client_url,
            client_auth,
            use_base64: _,
            use_mock,
            use_txn,
            compiled,
            outpoint,
            ordinals_info,
        } = self;
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
            .map(|b| deserialize::<PartiallySignedTransaction>(&b))
            .transpose()?;
        if let Some(psbt) = &use_txn {
            sapio_psbt::validate_psbt(psbt)?;
        }
        let client = rpc::Client::new(client_url, client_auth).await?;
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
                Some(PartiallySignedTransaction::from_unsigned_tx(tx.clone())?)
            } else {
                None
            };
            (tx, 0, psbt)
        } else if let Some(outpoint) = outpoint {
            let res = client.get_raw_transaction(&outpoint.txid, None).await?;
            validate_funding_outpoint(&res, outpoint)?;
            (res, outpoint.vout, None)
        } else {
            let mut spends = HashMap::new();
            let script = bitcoin::Script::from(&compiled.address);
            if let Some(a) = bitcoin::Address::from_script(&script, net) {
                spends.insert(format!("{}", a), compiled.required_input_amount);

                let psbt = if let Some(psbt) = use_txn {
                    psbt
                } else {
                    let res = client
                        .wallet_create_funded_psbt(&[], &spends, None, None, None)
                        .await?;
                    deserialize(&base64::decode(&res.psbt)?)?
                };
                let vout = funding_output(&psbt, &script)?;
                // Final scriptSigs can change the TXID. Bind the extracted
                // transaction while retaining the complete PSBT for signing.
                (psbt.clone().extract_tx(), vout, Some(psbt))
            } else {
                return Err(Err(RequestError("Must have a valid address".into()))?);
            }
        };
        let logger = Rc::new(TxIndexLogger::new());
        (*logger).add_tx(Arc::new(tx.clone()))?;
        let mut bound = compiled.bind_psbt(
            OutPoint::new(tx.txid(), vout),
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

fn funding_output(psbt: &PartiallySignedTransaction, script: &bitcoin::Script) -> ResultT<u32> {
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
    let actual = tx.txid();
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
