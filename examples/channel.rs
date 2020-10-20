use contract::*;
use sapio::*;

use crate::clause::{Clause, SATISIFIABLE, UNSATISIFIABLE};
use bitcoin;
use bitcoin::util::amount::Amount;
use ::miniscript::*;
use ::miniscript::policy::concrete::Policy;
use std::collections::HashMap;
use std::marker::PhantomData;
use schemars::{schema_for, JsonSchema};
use bitcoin::secp256k1::*;
use rand::OsRng;

fn main() {
    let full = Secp256k1::new();
    let mut rng = OsRng::new().expect("OsRng");
    let public_keys : Vec<_> = (0..3).map(|_| bitcoin::PublicKey{compressed: true, key:
        full.generate_keypair(&mut rng).1}).collect();
    let resolution =
        Compiled::from_descriptor(Descriptor::<bitcoin::PublicKey>::Pkh(public_keys[2]), None);

    let db = Arc::new(Mutex::new(MockDB{}));
    let x: Channel<Start> = Channel {
        pd: PhantomData,
        alice: public_keys[0],
        bob: public_keys[1],
        amount: Amount::from_sat(1),
        resolution: resolution.clone(),
        db:db.clone(),
    };
    let y: Channel<Stop> = Channel {
        pd: PhantomData,
        alice: public_keys[0],
        bob: public_keys[1],
        amount: Amount::from_sat(1),
        resolution,
        db:db.clone(),
    };
    Compilable::compile(&x);
    Compilable::compile(&y);
}

struct Start();
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
    Update{revoke: bitcoin::hashes::sha256::Hash, split: (Amount, Amount)}
}
trait DB {
   fn save(&self, a:Args);
}
struct MockDB {
}
impl DB for MockDB{
    fn save(&self, a:Args) {
        match a {
            Args::Update{..} =>
            {
                a;
            }
        }
    }
}

struct Channel<T: State> {
    pd: PhantomData<T>,
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    amount: Amount,
    resolution: Compiled,
    db: Arc<Mutex<DB>>
}

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
                            amount: s.amount,
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
