use contract::*;
use sapio::*;

use crate::clause::{Clause, SATISIFIABLE, UNSATISIFIABLE};
use ::miniscript::policy::concrete::Policy;
use ::miniscript::*;
use bitcoin;
use bitcoin::secp256k1::*;
use bitcoin::util::amount::{Amount, CoinAmount, ParseAmountError};
use rand::OsRng;
use schemars::{schema_for, JsonSchema};
use serde::{de::Visitor, Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;

fn main() {
    DB_serde::register_db("mock".to_string(), |s| Arc::new(Mutex::new(MockDB {})));
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

#[derive(JsonSchema)]
struct Start();
#[derive(JsonSchema)]
struct Stop();
trait State {}
impl State for Start {}
impl State for Stop {}
trait IsChannel<'a>
where
    Self: Sized + 'a,
{
    const begin_contest: Option<ThenFunc<'a, Self>> = None;
    const finish_contest: Option<ThenFunc<'a, Self>> = None;
}
use std::sync::{Arc, Mutex};

#[derive(Debug)]
enum Args {
    Update {
        revoke: bitcoin::hashes::sha256::Hash,
        split: (Amount, Amount),
    },
}
trait DB {
    fn save(&self, a: Args);
    fn link(&self) -> String;
}

#[derive(JsonSchema)]
struct MockDB {}
impl DB for MockDB {
    fn save(&self, a: Args) {
        match a {
            Args::Update { .. } => {
                a;
            }
        }
    }
    fn link(&self) -> String {
        format!("mock:{}", "name")
    }
}

mod DB_serde {
    use super::*;
    use std::fmt::{self, Display};

    use lazy_static::lazy_static;
    pub fn serialize<S>(db: &Arc<Mutex<dyn DB>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&db.lock().unwrap().link())
    }
    lazy_static! {
        static ref DB_TYPES: Mutex<HashMap<String, fn(&str) -> Arc<Mutex<dyn DB>>>> =
            Mutex::new(HashMap::new());
    }

    pub fn register_db(s: String, f: fn(&str) -> Arc<Mutex<dyn DB>>) {
        assert!(DB_TYPES.lock().unwrap().insert(s, f).is_none());
    }

    struct StringVisitor;
    impl<'de> Visitor<'de> for StringVisitor {
        type Value = Arc<Mutex<dyn DB>>;
        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("A handle, depending on what's locally registered")
        }
        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            if let Some(f) = DB_TYPES
                .lock()
                .unwrap()
                .get(value.split(":").next().unwrap())
            {
                Ok(f(value))
            } else {
                Err(E::unknown_variant(value, &[]))
            }
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Arc<Mutex<dyn DB>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(StringVisitor)
    }
}

#[derive(JsonSchema, Serialize, Deserialize)]
struct Channel<T: State> {
    pd: PhantomData<T>,
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    amount: CoinAmount,
    resolution: Compiled,
    #[schemars(with = "String")]
    #[serde(with = "DB_serde")]
    db: Arc<Mutex<dyn DB>>,
}

use std::convert::{TryFrom, TryInto};
type Coin = Result<Amount, ParseAmountError>;
impl<'a> IsChannel<'a> for Channel<Start> {
    then! {begin_contest |s| {
        Box::new(std::iter::once(
                txn::TemplateBuilder::new()
                .add_output(txn::Output::new(
                        s.amount,
                        Channel::<Stop> {
                            pd: Default::default(),
                            alice: s.alice,
                            bob: s.bob,
                            amount: s.amount.try_into().unwrap(),
                            resolution: s.resolution.clone(),
                            db: s.db.clone()
                        },
                        None,
                ))
                .into(),
        ))
    }
    }
}
impl<'a> IsChannel<'a> for Channel<Stop> {
    then! {finish_contest [Self::timeout] |s| {
        Box::new(std::iter::once(
                txn::TemplateBuilder::new()
                .add_output(txn::Output::new(s.amount, s.resolution.clone(), None))
                .into(),
        ))
    }}
}
impl<'a, T: State + 'a> Channel<T> {
    guard!(timeout | s | { Clause::Older(100) });
    guard!(cached signed |s| {Clause::And(vec![Clause::Key(s.alice), Clause::Key(s.bob)])});

    finish! {
        update_state_a [Self::signed]
            |s, o| {
                Box::new(std::iter::empty())
            }
    }
    finish! {
        update_state_b [Self::signed]
            |s, o| {
                Box::new(std::iter::empty())
            }
    }

    finish! {
        cooperate [Self::signed]
    }
}

impl<'a, T: State + 'a> Contract<'a> for Channel<T>
where
    Channel<T>: IsChannel<'a>,
{
    def! {then, Self::begin_contest, Self::finish_contest}
    def! {updatable<Args>, Self::update_state_a, Self::update_state_b }
    def! {finish, Self::signed}
}
