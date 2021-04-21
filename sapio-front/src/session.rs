// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! An interactive compilation session designed to be compatible with sapio-lang/TUX
use bitcoin::hashes::hex::ToHex;
use bitcoin::hashes::Hash;
use bitcoin::util::amount::Amount;
use sapio::contract::{Compilable, CompilationError, Compiled, Context};
use sapio::util::extended_address::ExtendedAddress;
use sapio_ctv_emulator_trait::CTVAvailable;
use schemars::schema::RootSchema;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt::Display;
use std::sync::Arc;

type Key = bitcoin::hashes::sha256::Hash;
/// Errors that can arise during a Session
#[derive(Debug)]
pub enum SessionError {
    /// Issue was with Serde
    Json(serde_json::Error),
    /// Issue came from Compilation
    Compiler(CompilationError),
    /// The session does not have an object saved for the key requested
    ContractNotRegistered,
}

impl std::error::Error for SessionError {}
impl Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        write!(f, "{:?}", self)
    }
}

impl From<std::convert::Infallible> for SessionError {
    fn from(_v: std::convert::Infallible) -> Self {
        panic!("Inhabited Never")
    }
}

impl From<CompilationError> for SessionError {
    fn from(v: CompilationError) -> Self {
        SessionError::Compiler(v)
    }
}

/// Create a compiled object of type `T` from a JSON
pub fn from_json<T>(s: serde_json::Value, ctx: &Context) -> Result<Compiled, SessionError>
where
    T: for<'a> Deserialize<'a> + Compilable,
{
    let t: T = serde_json::from_value(s).map_err(SessionError::Json)?;
    let c = ctx.compile(t).map_err(SessionError::Compiler);
    c
}

/// Create a compiled object of type `T` from a JSON which we first pass through
/// type `C`.
pub fn from_json_convert<C, T, E>(
    s: serde_json::Value,
    ctx: &Context,
) -> Result<Compiled, SessionError>
where
    C: for<'a> Deserialize<'a>,
    T: TryFrom<C, Error = E> + Compilable,
    SessionError: From<E>,
{
    let t: C = serde_json::from_value(s).map_err(SessionError::Json)?;
    let c = ctx
        .compile(T::try_from(t).map_err(SessionError::from)?)
        .map_err(SessionError::Compiler);
    c
}

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
    program: Vec<Value>,
}
/// An action requested by the client
#[derive(Serialize, Deserialize)]
#[serde(tag = "action", content = "content")]
enum Action {
    #[serde(rename = "close")]
    Close,
    #[serde(rename = "create")]
    Create {
        #[serde(rename = "type")]
        type_: String,
        args: Value,
    },
    #[serde(rename = "save")]
    Save(bitcoin::Address),
    #[serde(rename = "bind")]
    Bind(bitcoin::OutPoint, bitcoin::Address),
}

/// A response to a client request
#[derive(Serialize, Deserialize)]
#[serde(tag = "action", content = "content")]
pub enum Reaction {
    /// Send over a menu of available contracts / their arguments
    #[serde(rename = "menu")]
    Menu(Value),
    ///  sendthe Session ID
    #[serde(rename = "session_id")]
    Session(bool, String),
    /// Send the program created
    #[serde(rename = "created")]
    Created(
        #[serde(with = "bitcoin::util::amount::serde::as_sat")] Amount,
        ExtendedAddress,
        Program,
    ),
    /// if the save request completed successfully
    #[serde(rename = "saved")]
    Saved(bool),
    /// respond to Bind request with the transactions created
    #[serde(rename = "bound")]
    Bound(Vec<bitcoin::Transaction>),
}
fn create_mock_output() -> bitcoin::OutPoint {
    bitcoin::OutPoint {
        txid: bitcoin::hashes::sha256d::Hash::from_inner(
            bitcoin::hashes::sha256::Hash::hash(format!("mock:{}", 0).as_bytes()).into_inner(),
        )
        .into(),
        vout: 0,
    }
}

impl Action {
    fn react(self, session: &mut Session) -> Option<Reaction> {
        match self {
            Action::Close => None,
            Action::Create { type_, args } => {
                let c = session
                    .menu
                    .compile(type_, args, &session.get_context())
                    .ok()?;
                let a = c.address.clone();
                // todo amount
                let (txns, metadata) = c.bind(create_mock_output());
                let program = Program {
                    program: txns
                        .iter()
                        .map(bitcoin::consensus::encode::serialize)
                        .zip(metadata.into_iter())
                        .map(|(h, mut v)| {
                            v.as_object_mut()
                                .map(|ref mut m| m.insert("hex".into(), h.to_hex().into()));
                            v
                        })
                        .collect(),
                };
                println!("{:?}", program);
                Some(Reaction::Created(c.amount_range.max(), a, program))
            }
            Action::Save(_address) => Some(Reaction::Saved(true)),
            Action::Bind(_out, _address) => Some(Reaction::Bound(vec![])),
        }
    }
}

/// A struct for creating a session Menu interactively
pub struct MenuBuilder {
    menu: Vec<RootSchema>,
    gen: schemars::gen::SchemaGenerator,
    internal_menu: HashMap<String, fn(Value, &Context) -> Result<Compiled, SessionError>>,
    schemas: HashMap<String, String>,
}
impl MenuBuilder {
    /// create an empty Menu
    pub fn new() -> MenuBuilder {
        MenuBuilder {
            menu: Vec::new(),
            gen: schemars::gen::SchemaGenerator::default(),
            internal_menu: HashMap::new(),
            schemas: HashMap::new(),
        }
    }
    /// register type T with an optional name.
    /// If no name is provided, infer it from the type.
    pub fn register_as<T: JsonSchema + for<'a> Deserialize<'a> + Compilable>(
        &mut self,
        name: Option<String>,
    ) {
        let mut s = self.gen.root_schema_for::<T>();
        let title: &mut Option<String> = &mut s.schema.metadata().title;
        if name.is_some() {
            *title = name;
        }
        self.internal_menu
            .insert(title.clone().unwrap(), from_json::<T>);
        self.schemas.insert(
            title.clone().unwrap(),
            serde_json::to_string_pretty(&s).unwrap(),
        );
        self.menu.push(s);
    }
    /// register a type T with an optional name and a conversion type C.
    /// If no name is provided, infer it from the type C.
    pub fn register_as_from<
        C: JsonSchema + for<'a> Deserialize<'a>,
        T: Compilable + TryFrom<C, Error = E>,
        E,
    >(
        &mut self,
        name: Option<String>,
    ) where
        SessionError: From<E>,
    {
        let mut s = self.gen.root_schema_for::<C>();
        let title: &mut Option<String> = &mut s.schema.metadata().title;
        if name.is_some() {
            *title = name;
        }
        self.internal_menu
            .insert(title.clone().unwrap(), from_json_convert::<C, T, E>);
        self.schemas.insert(
            title.clone().unwrap(),
            serde_json::to_string_pretty(&s).unwrap(),
        );
        self.menu.push(s);
    }
    fn gen_menu(&self) -> Value {
        json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "oneOf": self.menu.iter().cloned().map(|x| {
            x
        }).collect::<Vec<RootSchema>>(),

        })
    }
    fn open(&self) -> Reaction {
        Reaction::Menu(self.gen_menu())
    }
}
impl From<MenuBuilder> for Menu {
    fn from(m: MenuBuilder) -> Self {
        Menu {
            menu: serde_json::to_string_pretty(&m.open()).unwrap(),
            internal_menu: m.internal_menu,
            schemas: m.schemas,
        }
    }
}

/// A precompiled menu of available contract options
pub struct Menu {
    menu: String,
    internal_menu: HashMap<String, fn(Value, &Context) -> Result<Compiled, SessionError>>,
    schemas: HashMap<String, String>,
}
impl Menu {
    /// create an instance of contract `name` with the provided args.
    pub fn compile(
        &self,
        name: String,
        args: Value,
        ctx: &Context,
    ) -> Result<Compiled, SessionError> {
        let f = self
            .internal_menu
            .get(&name)
            .ok_or(SessionError::ContractNotRegistered)?;
        f(args, ctx)
    }
    /// list all available contract names
    pub fn list(&self) -> impl Iterator<Item = &String> {
        self.internal_menu.keys()
    }
    /// get the schema for a particular contract
    pub fn schema_for(&self, name: &str) -> Option<&String> {
        self.schemas.get(name)
    }
}

/// An interactive compiler session
pub struct Session {
    contracts: HashMap<Key, Compiled>,
    example_msg: Option<String>,
    menu: &'static Menu,
    network: bitcoin::Network,
}

/// Internal msg type to permit either strings or bytes
pub enum Msg<'a> {
    /// msg as bytes
    Bytes(&'a [u8]),
    /// msg as string
    Text(&'a String),
}

impl Session {
    /// create an instance of a session with a fixed menu and a given network
    pub fn new(menu: &'static Menu, network: bitcoin::Network) -> Session {
        Session {
            contracts: HashMap::new(),
            example_msg: None,
            menu,
            network,
        }
    }
    /// get a context for this session
    /// TODO: link to a bitcoin node or something to determine available funds
    /// TODO: use an emulator if desired?
    pub fn get_context(&self) -> Context {
        // Todo: Make Create specify the amount to send.
        Context::new(
            self.network,
            Amount::from_sat(100_000_000_000),
            Arc::new(CTVAvailable),
        )
    }

    /// process a message from the Session manager (e.g., networking stack)
    /// and react to it.
    pub fn handle(&mut self, m: Msg) -> Result<Option<Reaction>, serde_json::Error> {
        let action: Action = match m {
            Msg::Text(m) => serde_json::from_str(&m),
            Msg::Bytes(m) => serde_json::from_slice(&m),
        }?;
        Ok(action.react(self))
    }

    /// returns the precompiled menu
    pub fn open(&mut self) -> &str {
        &self.menu.menu
    }
}
