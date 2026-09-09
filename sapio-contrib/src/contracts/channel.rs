// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! An example of how one might begin building a payment channel contract in Sapio
use bitcoin;
use bitcoin::util::amount::CoinAmount;
use contract::*;

use sapio::*;
use sapio_base::Clause;
use sapio_macros::guard;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::convert::TryFrom;
use std::convert::TryInto;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};
/// Helper

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{context, key};
    use bitcoin::Amount;

    fn channel() -> Channel<Start, Args> {
        Channel {
            pd: PhantomData,
            alice: key(1),
            bob: key(2),
            amount: Amount::from_sat(1000).into(),
            resolution: key(3).compile(context(1000)).unwrap(),
            db: Arc::new(Mutex::new(MockDB {})),
        }
    }

    #[test]
    fn contest_compiles_and_preserves_timeout_and_balance() {
        let object = channel().compile(context(1000)).unwrap();
        object.validate().unwrap();
        let start = object.ctv_to_tx.values().next().unwrap();
        assert_eq!(start.outputs[0].amount.as_sat(), 1000);
        let stop = start.outputs[0].contract.ctv_to_tx.values().next().unwrap();
        assert_eq!(stop.outputs[0].amount.as_sat(), 1000);
        assert_eq!(stop.tx.input[0].sequence, 100);
        assert!(serde_json::to_string(&start.outputs[0].contract.descriptor)
            .unwrap()
            .contains("older(100)"));
    }

    #[test]
    fn cooperative_close_requires_both_keys_and_conserves_value() {
        let contract = channel();
        assert_eq!(
            contract.guard_signed(),
            Clause::And(vec![Clause::Key(key(1)), Clause::Key(key(2))])
        );
        let update = |a, b| {
            Some(Update {
                split: (Amount::from_sat(a).into(), Amount::from_sat(b).into()),
            })
        };
        let template = contract
            .continue_cooperate(context(1000), update(400, 600))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(
            template
                .tx
                .output
                .iter()
                .map(|o| o.value)
                .collect::<Vec<_>>(),
            vec![400, 600]
        );
        assert!(contract
            .continue_cooperate(context(1000), update(400, 601))
            .is_err());
        assert!(contract
            .continue_cooperate(context(1000), None)
            .unwrap()
            .next()
            .is_none());
        assert!(contract.compile(context(999)).is_err());
    }

    #[test]
    fn database_locator_round_trips_without_phantom_arguments() {
        db_serde::register_db("mock".into(), |_| Arc::new(Mutex::new(MockDB {})));
        let json = serde_json::to_string(&channel()).unwrap();
        assert!(!json.contains("pd"));
        let decoded: Channel<Start, Args> = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.alice, key(1));
        assert_eq!(decoded.db.lock().unwrap().link().type_, "mock");
    }
}

/// Balances for an authenticated cooperative channel close.
#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct Update {
    /// the balances of the channel
    split: (CoinAmount, CoinAmount),
}
impl TryFrom<Args> for Update {
    type Error = CompilationError;
    fn try_from(a: Args) -> Result<Update, CompilationError> {
        if let Args::Update(u) = a {
            Ok(u)
        } else {
            Err(CompilationError::Custom("Unmatched".into()))
        }
    }
}
/// Args are some messages that can be passed to a Channel instance
#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub enum Args {
    /// Wrapper around Update
    Update(Update),
    /// No cooperative settlement update was supplied.
    None,
}
impl Default for Args {
    fn default() -> Self {
        Args::None
    }
}
impl StatefulArgumentsTrait for Args {}

/// Handle for DB Types
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct DBHandle {
    type_: String,
    id: String,
}
/// DB Trait is for a Trait Object that can be used to record state updates for a channel.
/// Examples implements a MockDB
pub trait DB {
    /// Simply save a transcript of all messages to reconstrue channel state
    fn save(&self, a: Args);
    /// gets a handle to this DB instance for global lookup
    fn link(&self) -> DBHandle;
}

#[derive(JsonSchema)]
struct MockDB {}
impl DB for MockDB {
    fn save(&self, a: Args) {
        match a {
            Args::Update { .. } => {}
            Args::None => {}
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
        static ref DB_TYPES: Mutex<BTreeMap<String, fn(&str) -> Arc<Mutex<dyn DB>>>> =
            Mutex::new(BTreeMap::new());
    }

    pub fn register_db(s: String, f: fn(&str) -> Arc<Mutex<dyn DB>>) {
        assert!(DB_TYPES.lock().unwrap().insert(s, f).is_none());
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Arc<Mutex<dyn DB>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let handle = DBHandle::deserialize(deserializer)?;
        let resolver = DB_TYPES.lock().unwrap().get(&handle.type_).copied();
        if let Some(f) = resolver {
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
/// State Start
#[derive(JsonSchema)]
struct Start();
/// state Stop
#[derive(JsonSchema)]
struct Stop();
impl State for Start {}
impl State for Stop {}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(bound(serialize = "", deserialize = ""))]
struct Channel<T: State, ArgsT: TryInto<Update>> {
    #[serde(skip, default)]
    pd: PhantomData<(T, ArgsT)>,
    #[schemars(with = "String")]
    alice: bitcoin::XOnlyPublicKey,
    #[schemars(with = "String")]
    bob: bitcoin::XOnlyPublicKey,
    amount: CoinAmount,
    resolution: Compiled,
    /// We instruct the JSONSchema to use strings
    #[schemars(with = "DBHandle")]
    #[serde(with = "db_serde")]
    db: Arc<Mutex<dyn DB>>,
}

fn coerce_args(t: Args) -> Result<Option<Update>, CompilationError> {
    Ok(match t {
        Args::Update(update) => Some(update),
        Args::None => None,
    })
}

/// Functionality Available for a channel regardless of state
impl<T: State> Channel<T, Args>
where
    Self: Contract<StatefulArguments = Args>,
{
    #[guard]
    fn timeout(self, _ctx: Context) {
        Clause::Older(100)
    }
    #[guard(cached)]
    fn signed(self) {
        Clause::And(vec![Clause::Key(self.alice), Clause::Key(self.bob)])
    }

    #[continuation(guarded_by = "[Self::signed]", coerce_args = "coerce_args", web_api)]
    fn cooperate(self, ctx: sapio::Context, update: Option<Update>) {
        let Some(update) = update else { return empty() };
        let alice: bitcoin::Amount = update.split.0.try_into()?;
        let bob: bitcoin::Amount = update.split.1.try_into()?;
        if alice.checked_add(bob) != Some(self.amount.try_into()?) {
            return Err(CompilationError::Custom(
                "Channel close must preserve its balance".into(),
            ));
        }
        let mut template = ctx.template();
        if alice != bitcoin::Amount::ZERO {
            template = template.add_output(alice, &self.alice, None)?;
        }
        if bob != bitcoin::Amount::ZERO {
            template = template.add_output(bob, &self.bob, None)?;
        }
        template.into()
    }
}

/// Functionality that differs depending on current State
trait FunctionalityAtState
where
    Self: Sized + Contract,
    <Self as Contract>::StatefulArguments: TryInto<Update>,
{
    decl_then! {begin_contest}
    decl_then! {finish_contest}
}

/// Override begin_contest when state = Start
impl FunctionalityAtState for Channel<Start, Args> {
    #[then]
    fn begin_contest(self, ctx: sapio::Context) {
        ctx.template()
            .add_output(
                self.amount.try_into()?,
                &Channel::<Stop, Args> {
                    pd: Default::default(),
                    alice: self.alice,
                    bob: self.bob,
                    amount: self.amount,
                    resolution: self.resolution.clone(),
                    db: self.db.clone(),
                },
                None,
            )?
            .into()
    }
}

/// Override finish_contest when state = Start
impl FunctionalityAtState for Channel<Stop, Args> {
    #[then(guarded_by = "[Self::timeout]")]
    fn finish_contest(self, ctx: sapio::Context) {
        ctx.template()
            .add_output(self.amount.try_into()?, &self.resolution, None)?
            .set_sequence(0, sapio_base::timelocks::RelHeight::from(100).into())?
            .into()
    }
}

/// Implement Contract for Channel<T> and functionality will be correctly assembled for different
/// States.
impl Contract for Channel<Start, Args> {
    declare! {then, Self::begin_contest, Self::finish_contest}
    declare! {updatable<Args>, Self::cooperate }
    declare! {finish, Self::signed}
}

impl Contract for Channel<Stop, Args> {
    declare! {then, Self::begin_contest, Self::finish_contest}
    declare! {updatable<Args>, Self::cooperate }
    declare! {finish, Self::signed}
}
