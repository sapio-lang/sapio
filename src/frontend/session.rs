type Key = bitcoin::hashes::sha256::Hash;
use crate::contract::{Compilable, CompilationError, Compiled, Contract};
use bitcoin::util::amount::CoinAmount;
use schemars::schema::{RootSchema, Schema, SchemaObject};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

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

#[derive(Serialize, Deserialize)]
#[serde(tag = "action", content = "content")]
pub enum Reaction {
    #[serde(rename = "menu")]
    Menu(Value),
    #[serde(rename = "session_id")]
    Session(bool, String),
    #[serde(rename = "created")]
    Created(CoinAmount, bitcoin::Address),
    #[serde(rename = "saved")]
    Saved(bool),
    #[serde(rename = "bound")]
    Bound(Vec<bitcoin::Transaction>),
}

impl Action {
    fn react(self, session: &mut Session) -> Option<Reaction> {
        match self {
            Action::Close => None,
            Action::Create { type_, args } => {
                let c = session.menu.compile(type_, args).ok()?;
                let a = c.descriptor.address(bitcoin::Network::Bitcoin)?;
                // todo amount
                Some(Reaction::Created(c.amount_range.max(), a))
            }
            Action::Save(address) => Some(Reaction::Saved(true)),
            Action::Bind(out, address) => Some(Reaction::Bound(vec![])),
        }
    }
}

pub struct MenuBuilder {
    menu: Vec<RootSchema>,
    gen: schemars::gen::SchemaGenerator,
    internal_menu: HashMap<String, fn(Value) -> Result<Compiled, CompilationError>>,
}
impl MenuBuilder {
    pub fn new() -> MenuBuilder {
        MenuBuilder {
            menu: Vec::new(),
            gen: schemars::gen::SchemaGenerator::default(),
            internal_menu: HashMap::new(),
        }
    }
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
            .insert(title.clone().unwrap(), <T as Compilable>::from_json);
        self.menu.push(s);
    }
    fn gen_menu(&self) -> Value {
        json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "oneOf": self.menu.iter().cloned().map(|mut x| {
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
            menu: serde_json::to_string(&m.open()).unwrap(),
            internal_menu: m.internal_menu,
        }
    }
}

pub struct Menu {
    menu: String,
    internal_menu: HashMap<String, fn(Value) -> Result<Compiled, CompilationError>>,
}
impl Menu {
    fn compile(&self, name: String, args: Value) -> Result<Compiled, CompilationError> {
        let f = self
            .internal_menu
            .get(&name)
            .ok_or(CompilationError::TerminateCompilation)?;
        f(args)
    }
}

pub struct Session {
    contracts: HashMap<Key, Compiled>,
    example_msg: Option<String>,
    menu: &'static Menu,
}

pub enum Msg {
    Bytes(actix_web::web::Bytes),
    Text(String),
}

impl Session {
    pub fn new(m: &'static Menu) -> Session {
        Session {
            contracts: HashMap::new(),
            example_msg: None,
            menu: m,
        }
    }

    pub fn handle(&mut self, m: Msg) -> Result<Option<Reaction>, serde_json::Error> {
        let action: Action = match m {
            Msg::Text(m) => serde_json::from_str(&m),
            Msg::Bytes(m) => serde_json::from_slice(&m),
        }?;
        Ok(action.react(self))
    }

    pub fn open(&mut self) -> &str {
        &self.menu.menu
    }
}
