// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
use bitcoin::{consensus::Decodable, psbt::PartiallySignedTransaction, OutPoint};
use bitcoincore_rpc_async as rpc;
use bitcoincore_rpc_async::RpcApi;
use emulator_connect::{CTVAvailable, CTVEmulator};
use sapio::{
    contract::{
        object::{LinkedPSBT, ObjectMetadata, Program, SapioStudioObject},
        Compiled,
    },
    template::{OutputMeta, TemplateMetadata},
    util::extended_address::ExtendedAddress,
    Context,
};
use sapio_base::{
    effects::{MapEffectDB, PathFragment},
    serialization_helpers::SArc,
    txindex::{TxIndex, TxIndexLogger},
};
use sapio_wasm_plugin::{
    host::{plugin_handle::ModuleLocator, PluginHandle, WasmPluginHandle},
    CreateArgs, API,
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

use crate::{config::EmulatorConfig, util::create_mock_output};

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct Common {
    pub path: PathBuf,
    pub emulator: Option<EmulatorConfig>,
    pub module_locator: ModuleLocator,
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
    async fn get_emulator(&self) -> ResultT<Arc<dyn CTVEmulator>> {
        let emulator: Arc<dyn CTVEmulator> = if let Some(emcfg) = &self.context.emulator {
            if emcfg.enabled {
                emcfg.get_emulator()?.into()
            } else {
                Arc::new(CTVAvailable)
            }
        } else {
            Arc::new(CTVAvailable)
        };
        Ok(emulator)
    }
    pub async fn handle(self) -> Response {
        let v = self.handle_inner().await.map_err(|e| -> RequestError {
            e.downcast::<RequestError>()
                .map(|d| *d)
                .unwrap_or_else(|e| RequestError(e.to_string().into()))
        });
        Response { result: v }
    }
    pub async fn handle_inner(self) -> ResultT<CommandReturn> {
        let emulator = self.get_emulator().await?;
        // create the future to get the sph,
        // but do not await it since not all calls will use it.
        let Request { context, command } = self;
        let Common {
            path,
            module_locator,
            net,
            plugin_map,
            ..
        } = context;
        let default_sph = || {
            WasmPluginHandle::<Value>::new_async(
                &path,
                &emulator,
                module_locator,
                net,
                plugin_map.clone(),
            )
        };
        match command {
            Command::List(_list) => {
                let plugins = WasmPluginHandle::<Value>::load_all_keys(
                    &path,
                    emulator.clone(),
                    context.net,
                    plugin_map,
                )?;
                let m = plugins
                    .iter()
                    .map(|p| p.get_name().map(|name| (p.id().to_string(), name)))
                    .collect::<Result<BTreeMap<_, _>, _>>()?;
                Ok(CommandReturn::List(ListReturn { items: m }))
            }
            Command::Call(call) => {
                let params = call.params;
                let sph = default_sph().await?;

                let api = sph.get_api()?;
                let schema = serde_json::to_value(api.input())?;
                let validator = jsonschema_valid::Config::from_schema(
                    &schema,
                    Some(jsonschema_valid::schemas::Draft::Draft6),
                )?;
                if let Err(it) = validator.validate(&params) {
                    let mut w = String::new();
                    for err in it {
                        writeln!(w, "Error: {}", err)?;
                    }
                    // returns
                    Err(RequestError(w.into()))?;
                }
                let create_args: CreateArgs<serde_json::Value> = serde_json::from_value(params)?;
                let v = sph.call(&PathFragment::Root.into(), &create_args)?;
                Ok(CommandReturn::Call(CallReturn { result: v }))
            }
            Command::Bind(bind) => Ok(CommandReturn::Bind(bind.call(net, emulator).await?)),
            Command::Api(_api) => {
                let sph = default_sph().await?;
                Ok(CommandReturn::Api(ApiReturn {
                    api: sph.get_api()?,
                }))
            }
            Command::Logo(_logo) => {
                let sph = default_sph().await?;
                Ok(CommandReturn::Logo(LogoReturn {
                    logo: sph.get_logo()?.into(),
                }))
            }
            Command::Info(_info) => {
                let sph = default_sph().await?;
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
                let sph = default_sph().await?;
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
    ) -> Result<BindReturn, Box<dyn Error>> {
        let Bind {
            client_url,
            client_auth,
            use_base64: _,
            use_mock,
            use_txn,
            compiled,
            outpoint,
        } = self;
        let use_txn = use_txn
            .map(|buf| base64::decode(buf.as_bytes()))
            .transpose()?
            .map(|b| PartiallySignedTransaction::consensus_decode(&b[..]))
            .transpose()?;
        let client = rpc::Client::new(client_url, client_auth).await?;
        let (tx, vout) = if use_mock {
            let ctx = Context::new(
                net,
                compiled.amount_range.max(),
                emulator.clone(),
                "mock".try_into()?,
                Arc::new(MapEffectDB::default()),
            );
            let mut tx = ctx
                .template()
                .add_output(compiled.amount_range.max(), &compiled, None)?
                .get_tx();
            tx.input[0].previous_output = create_mock_output();
            (tx, 0)
        } else if let Some(outpoint) = outpoint {
            let res = client.get_raw_transaction(&outpoint.txid, None).await?;
            (res, outpoint.vout)
        } else {
            let mut spends = HashMap::new();
            if let ExtendedAddress::Address(ref a) = compiled.address {
                spends.insert(format!("{}", a), compiled.amount_range.max());

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
                        return Err(Err(RequestError(
                            format!("No Output found {:?} {:?}", psbt.unsigned_tx, a).into(),
                        ))?);
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
                return Err(Err(RequestError("Must have a valid address".into()))?);
            }
        };
        let logger = Rc::new(TxIndexLogger::new());
        (*logger).add_tx(Arc::new(tx.clone()))?;
        let mut bound = compiled.bind_psbt(
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
        Ok(bound)
    }
}
