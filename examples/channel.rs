use contract::actions::*;
use contract::*;
use sapio::*;

use crate::clause::Clause;

use ::miniscript::*;
use bitcoin;
use bitcoin::secp256k1::*;
use bitcoin::util::amount::{Amount, CoinAmount};
use rand::rngs::OsRng;
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;

use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use std::convert::TryInto;

fn main() {
    db_serde::register_db("mock".to_string(), |_s| Arc::new(Mutex::new(MockDB {})));
    let full = Secp256k1::new();
    let mut rng = OsRng::new().expect("OsRng");
    let public_keys: Vec<_> = (0..3)
        .map(|_| bitcoin::PublicKey {
            compressed: true,
            key: full.generate_keypair(&mut rng).1,
        })
        .collect();
    let resolution =
        Compiled::from_descriptor(Descriptor::<bitcoin::PublicKey>::Pkh(public_keys[2]), None);

    let db = Arc::new(Mutex::new(MockDB {}));
    let x: Channel<Start> = Channel {
        pd: PhantomData,
        alice: public_keys[0],
        bob: public_keys[1],
        amount: Amount::from_sat(1).into(),
        resolution: resolution.clone(),
        db: db.clone(),
    };
    let y: Channel<Stop> = Channel {
        pd: PhantomData,
        alice: public_keys[0],
        bob: public_keys[1],
        amount: Amount::from_sat(1).into(),
        resolution,
        db: db.clone(),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&schema_for!(Channel<Stop>)).unwrap()
    );
    println!("{}", serde_json::to_string_pretty(&y).unwrap());
    Compilable::compile(&x);
    Compilable::compile(&y);
}

/// Args are some messages that can be passed to a Channel instance
#[derive(Debug)]
pub enum Args {
    Update {
        revoke: bitcoin::hashes::sha256::Hash,
        split: (Amount, Amount),
    },
}

/// Handle for DB Types
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct DBHandle {
    type_: String,
    id: String,
}
/// DB Trait is for a Trait Object that can be used to record state updates for a channel.
/// Examples implements a MockDB
pub trait DB {
    fn save(&self, a: Args);
    fn link(&self) -> DBHandle;
}

#[derive(JsonSchema)]
struct MockDB {}
impl DB for MockDB {
    fn save(&self, a: Args) {
        match a {
            Args::Update { .. } => {
            }
        }
    }
    fn link(&self) -> DBHandle {
        DBHandle {
            type_: "mock".into(),
            id: "".into(),
        }
    }
}

/// Custom Serialization Logic for DB Trait Critically, the method register_db can be used to add
/// resolvers to get references to DB instances of arbitrary types.
mod db_serde {
    use super::*;
    use serde::de::Error;

    use lazy_static::lazy_static;
    lazy_static! {
        static ref DB_TYPES: Mutex<HashMap<String, fn(&str) -> Arc<Mutex<dyn DB>>>> =
            Mutex::new(HashMap::new());
    }

    pub fn register_db(s: String, f: fn(&str) -> Arc<Mutex<dyn DB>>) {
        assert!(DB_TYPES.lock().unwrap().insert(s, f).is_none());
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Arc<Mutex<dyn DB>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let handle = DBHandle::deserialize(deserializer)?;
        if let Some(f) = DB_TYPES.lock().unwrap().get(&handle.type_) {
            Ok(f(&handle.id))
        } else {
            Err(D::Error::unknown_variant(&handle.type_, &[]))
        }
    }

    pub fn serialize<S>(db: &Arc<Mutex<dyn DB>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        db.lock().unwrap().link().serialize(serializer)
    }
}

/// The Different Operating States a Channel may be in.
/// These States are enum'd at the trait/type level so as
/// to be used as type tags
trait State {}
#[derive(JsonSchema)]
struct Start();
#[derive(JsonSchema)]
struct Stop();
impl State for Start {}
impl State for Stop {}

#[derive(JsonSchema, Serialize, Deserialize)]
struct Channel<T: State> {
    pd: PhantomData<T>,
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    amount: CoinAmount,
    resolution: Compiled,
    /// We instruct the JSONSchema to use strings
    #[schemars(with = "DBHandle")]
    #[serde(with = "db_serde")]
    db: Arc<Mutex<dyn DB>>,
}

/// Functionality Available for a channel regardless of state
impl<'a, T: State + 'a> Channel<T> {
    guard!(timeout | s | { Clause::Older(100) });
    guard!(cached signed |s| {Clause::And(vec![Clause::Key(s.alice), Clause::Key(s.bob)])});

    finish! {
        update_state_a [Self::signed]
            |s, o| {
                Ok(Box::new(std::iter::empty()))
            }
    }
    finish! {
        update_state_b [Self::signed]
            |s, o| {
                Ok(Box::new(std::iter::empty()))
            }
    }

    finish! {
        cooperate [Self::signed]
    }
}

/// Functionality that differs depending on current State
trait FunctionalityAtState<'a>
where
    Self: Sized + 'a,
{
    fn begin_contest() -> Option<ThenFunc<'a, Self>> {
        None
    }
    fn finish_contest() -> Option<ThenFunc<'a, Self>> {
        None
    }
}

/// Override begin_contest when state = Start
impl<'a> FunctionalityAtState<'a> for Channel<Start> {
    then! {begin_contest |s| {
        let o = txn::Output::new( s.amount,
            Channel::<Stop> {
                pd: Default::default(),
                alice: s.alice,
                bob: s.bob,
                amount: s.amount.try_into().unwrap(),
                resolution: s.resolution.clone(),
                db: s.db.clone()
            },
            None)?;
        Ok(Box::new(std::iter::once(
                    txn::TemplateBuilder::new()
                    .add_output(o).into())
        ))
    }
    }
}

/// Override finish_contest when state = Start
impl<'a> FunctionalityAtState<'a> for Channel<Stop> {
    then! {finish_contest [Self::timeout] |s| {
        let o =  txn::Output::new( s.amount, s.resolution.clone(), None)?;
        Ok(Box::new(std::iter::once(
                txn::TemplateBuilder::new()
                          .add_output(o)
                          .into()
        )))
    }}
}

/// Implement Contract for Channel<T> and functionality will be correctly assembled for different
/// States.
impl<'a, T: State + 'a> Contract<'a> for Channel<T>
where
    Channel<T>: FunctionalityAtState<'a>,
{
    def! {then, Self::begin_contest, Self::finish_contest}
    def! {updatable<Args>, Self::update_state_a, Self::update_state_b }
    def! {finish, Self::signed}
}
