use crate::clause::Clause;
use crate::core::template::Output;
use crate::core::template::{Builder, Template};
use crate::*;
use contract::actions::*;
use contract::*;

use ::miniscript::*;
use bitcoin;
use bitcoin::secp256k1::*;
use bitcoin::util::amount::{Amount, CoinAmount};
use rand::rngs::OsRng;
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;

use std::marker::PhantomData;

use std::convert::TryFrom;
use std::convert::TryInto;

use std::rc::Rc;

pub mod oracle;
pub use oracle::{Oracle, Symbol};

pub mod apis;
pub mod call;
pub mod exploding;
pub mod put;
pub mod risk_reversal;

/// To setup a GenericBet select an amount, a list of outcomes, and an oracle.
/// The outcomes do not need to be sorted but must be unique.
struct GenericBetArguments<'a> {
    amount: Amount,
    outcomes: Vec<(i64, Template)>,
    oracle: &'a dyn Oracle,
    cooperate: Clause,
    symbol: Symbol,
}

/// We can then convert the arguments into a specific contract instance
impl<'a> From<GenericBetArguments<'a>> for GenericBet {
    fn from(mut v: GenericBetArguments<'a>) -> GenericBet {
        // Make sure the outcomes are sorted for the binary tree
        v.outcomes.sort_by_key(|(i, _)| *i);
        // Cache locally all calls to the oracle
        let mut h = HashMap::new();
        for (k, _) in v.outcomes.iter() {
            let r = v.oracle.get_key_lt_gte(&v.symbol, *k);
            h.insert(*k, r);
        }
        GenericBet {
            amount: v.amount,
            outcomes: v.outcomes,
            oracle: Rc::new(h),
            cooperate: v.cooperate,
        }
    }
}

/// A GenericBet takes a sorted list of outcomes and a cached table of
/// oracle lookups and assembles a binary contract tree for the GenericBet
struct GenericBet {
    amount: Amount,
    outcomes: Vec<(i64, Template)>,
    oracle: Rc<HashMap<i64, (Clause, Clause)>>,
    cooperate: Clause,
}

impl GenericBet {
    /// The oracle price kyes for this part of the tree is in the middle of the range.
    fn price(&self, b: bool) -> Clause {
        let v = &self.oracle[&self.outcomes[self.outcomes.len() / 2].0];
        if b {
            v.1.clone()
        } else {
            v.0.clone()
        }
    }
    fn recurse_over(
        &self,
        range: std::ops::Range<usize>,
    ) -> Result<Option<Template>, CompilationError> {
        match &self.outcomes[range] {
            [] => return Ok(None),
            [(_, a)] => Ok(Some(a.clone())),
            sl => Ok(Some(
                template::Builder::new()
                    .add_output(Output::new(
                        self.amount.into(),
                        &GenericBet {
                            amount: self.amount,
                            outcomes: sl.into(),
                            oracle: self.oracle.clone(),
                            cooperate: self.cooperate.clone(),
                        },
                        None,
                    )?)
                    .into(),
            )),
        }
    }
    /// Action when the price is greater than or equal to the price in the middle
    guard!(gte | s | { s.price(true) });
    then!(
        pay_gte[Self::gte] | s | {
            if let Some(tmpl) = s.recurse_over(s.outcomes.len() / 2..s.outcomes.len())? {
                Ok(Box::new(std::iter::once(Ok(tmpl))))
            } else {
                Ok(Box::new(std::iter::empty()))
            }
        }
    );

    /// Action when the price is less than or equal to the price in the middle
    guard!(lt | s | { s.price(false) });
    then!(
        pay_lt[Self::lt] | s | {
            if let Some(tmpl) = s.recurse_over(0..s.outcomes.len() / 2)? {
                Ok(Box::new(std::iter::once(Ok(tmpl))))
            } else {
                Ok(Box::new(std::iter::empty()))
            }
        }
    );
    /// Allow for both parties to cooperative close
    guard!(cooperate | s | { s.cooperate.clone() });

    // elided: unilateral close initiation after certain relative delay
}

impl Contract for GenericBet {
    declare!(updatable<()>);
    declare!(then, Self::pay_gte, Self::pay_lt);
}
