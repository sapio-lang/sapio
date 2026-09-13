// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A collection of modules for creating derivative contracts with Sapio
use bitcoin;
use bitcoin::Amount;
use contract::*;
use sapio::template::Template;
use sapio::*;
use sapio_base::Clause;
use sapio_macros::guard;
use std::collections::BTreeMap;
use std::convert::TryFrom;
use std::rc::Rc;

pub mod oracle;
pub use oracle::{Oracle, Symbol};

pub mod apis;
pub mod call;
pub mod exploding;
pub mod powswap;
pub mod put;
pub mod risk_reversal;
pub mod signature_attested;

/// To setup a GenericBet select an amount, a list of outcomes, and an oracle.
/// The outcomes do not need to be sorted but must be unique.
pub struct GenericBetArguments<'a> {
    /// Collateral required by every settlement.
    pub amount: Amount,
    /// Lower price endpoints and settlements, with the first outcome extending below its endpoint.
    pub outcomes: Vec<(i64, Template)>,
    /// Oracle conditions for the interval boundaries.
    pub oracle: &'a dyn Oracle,
    /// Cooperative closing condition.
    pub cooperate: Clause,
    /// Oracle price symbol.
    pub symbol: Symbol,
}

/// We can then convert the arguments into a specific contract instance
impl<'a> TryFrom<GenericBetArguments<'a>> for GenericBet {
    type Error = CompilationError;
    fn try_from(mut v: GenericBetArguments<'a>) -> Result<GenericBet, Self::Error> {
        // Make sure the outcomes are sorted for the binary tree
        v.outcomes.sort_by_key(|(i, _)| *i);
        if v.outcomes.is_empty()
            || v.amount.to_sat() == 0
            || v.outcomes.windows(2).any(|p| p[0].0 == p[1].0)
            || v.outcomes
                .iter()
                .any(|(_, t)| t.max != v.amount || t.tx.input.len() != 1 || t.tx.output.is_empty())
        {
            return Err(invalid(
                "Bet outcomes must be nonempty, unique, and distribute the collateral",
            ));
        }
        // Cache locally all calls to the oracle
        let mut h = BTreeMap::new();
        for (k, _) in v.outcomes.iter() {
            let r = v.oracle.get_key_lt_gte(&v.symbol, *k);
            h.insert(*k, r);
        }
        Ok(GenericBet {
            amount: v.amount,
            outcomes: v.outcomes,
            oracle: Rc::new(h),
            cooperate: v.cooperate,
        })
    }
}

/// A GenericBet takes a sorted list of outcomes and a cached table of
/// oracle lookups and assembles a binary contract tree for the GenericBet
#[derive(Clone)]
pub struct GenericBet {
    amount: Amount,
    outcomes: Vec<(i64, Template)>,
    oracle: Rc<BTreeMap<i64, (Clause, Clause)>>,
    cooperate: Clause,
}

impl GenericBet {
    /// The oracle price keys for this part of the tree is in the middle of the range.
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
        ctx: sapio::contract::Context,
    ) -> Result<Option<Template>, CompilationError> {
        match &self.outcomes[range] {
            [] => Ok(None),
            [(_, a)] => Ok(Some(a.clone())),
            sl => Ok(Some(
                ctx.template()
                    .add_output(
                        self.amount,
                        &GenericBet {
                            amount: self.amount,
                            outcomes: sl.into(),
                            oracle: self.oracle.clone(),
                            cooperate: self.cooperate.clone(),
                        },
                        None,
                    )?
                    .into(),
            )),
        }
    }
    /// Action when the price is greater than or equal to the price in the middle
    #[guard]
    fn gte(self, _ctx: Context) {
        self.price(true)
    }
    #[then(guarded_by = "[Self::gte]")]
    fn pay_gte(self, ctx: sapio::Context) {
        if let Some(tmpl) = self.recurse_over(self.outcomes.len() / 2..self.outcomes.len(), ctx)? {
            Ok(Box::new(std::iter::once(Ok(tmpl))))
        } else {
            Ok(Box::new(std::iter::empty()))
        }
    }

    /// Action when the price is less than or equal to the price in the middle
    #[guard]
    fn lt(self, _ctx: Context) {
        self.price(false)
    }
    #[then(guarded_by = "[Self::lt]")]
    fn pay_lt(self, ctx: sapio::Context) {
        if let Some(tmpl) = self.recurse_over(0..std::cmp::max(1, self.outcomes.len() / 2), ctx)? {
            Ok(Box::new(std::iter::once(Ok(tmpl))))
        } else {
            Ok(Box::new(std::iter::empty()))
        }
    }
    /// Allow for both parties to cooperative close
    #[guard]
    fn cooperate(self, _ctx: Context) {
        self.cooperate.clone()
    }

    // elided: unilateral close initiation after certain relative delay
}

impl Contract for GenericBet {
    fn ensure_amount(&self, ctx: Context) -> Result<Amount, CompilationError> {
        if ctx.funds() < self.amount {
            return Err(CompilationError::OutOfFunds);
        }
        Ok(self.amount)
    }

    declare! {finish, Self::cooperate}
    declare! {actions, Self::pay_gte, Self::pay_lt}
}

fn invalid(message: &str) -> CompilationError {
    CompilationError::TerminateWith(message.into())
}

/// Price precision shared by the example option schedules.
pub const PRICE_UNIT: u64 = 10_000;

fn scaled_amount(
    amount: Amount,
    numerator: u64,
    denominator: u64,
) -> Result<Amount, CompilationError> {
    if denominator == 0 {
        return Err(invalid("Zero price denominator"));
    }
    let sats = (u128::from(amount.to_sat()) * u128::from(numerator)) / u128::from(denominator);
    Ok(Amount::from_sat(
        u64::try_from(sats).map_err(|_| invalid("Option amount overflow"))?,
    ))
}

fn settlement(
    ctx: Context,
    user_amount: Amount,
    operator_amount: Amount,
    user: &dyn apis::UserApi,
    operator: &dyn apis::OperatorApi,
) -> Result<Template, CompilationError> {
    let mut builder = ctx.template();
    if user_amount.to_sat() > 0 {
        builder = builder.add_output(user_amount, &user.receive_payment(user_amount), None)?;
    }
    if operator_amount.to_sat() > 0 {
        builder = builder.add_output(
            operator_amount,
            &operator.receive_payment(operator_amount),
            None,
        )?;
    }
    Ok(builder.into())
}

fn price_grid(bottom: u64, top: u64) -> Result<impl Iterator<Item = u64>, CompilationError> {
    if bottom > top || top > i64::MAX as u64 {
        return Err(invalid("Invalid oracle price range"));
    }
    // Include the cap even when the final interval is shorter than one unit.
    let endpoint = ((top - bottom) % PRICE_UNIT != 0).then_some(top);
    Ok((bottom..=top).step_by(PRICE_UNIT as usize).chain(endpoint))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{address, context, key};
    use sapio_base::miniscript::Threshold;
    struct PriceOracle;
    impl Oracle for PriceOracle {
        fn get_key_lt_gte(&self, _: &Symbol, price: i64) -> (Clause, Clause) {
            let n = (price / PRICE_UNIT as i64) as u8;
            (Clause::Key(key(n + 10)), Clause::Key(key(n + 20)))
        }
    }
    struct Operator;
    struct User;
    impl apis::OperatorApi for Operator {
        fn get_oracle(&self) -> &dyn Oracle {
            &PriceOracle
        }
        fn get_key(&self) -> Clause {
            Clause::Key(key(1))
        }
        fn receive_payment(&self, _: Amount) -> Compiled {
            Compiled::from_address(address(1), bitcoin::Amount::ZERO)
        }
    }
    impl apis::UserApi for User {
        fn get_key(&self) -> Clause {
            Clause::Key(key(2))
        }
        fn receive_payment(&self, _: Amount) -> Compiled {
            Compiled::from_address(address(2), bitcoin::Amount::ZERO)
        }
    }
    fn call(buying: bool, cap: u64, funds: u64) -> call::Call<'static> {
        call::Call {
            amount: Amount::from_sat(5000),
            strike_x_one_unit: 10_000,
            max_price_x_one_unit: cap,
            operator_api: &Operator,
            user_api: &User,
            symbol: "price".into(),
            buying,
            ctx: context(funds),
        }
    }
    fn put(buying: bool) -> put::Put<'static> {
        put::Put {
            amount: Amount::from_sat(5000),
            strike_x_one_unit: 20_000,
            operator_api: &Operator,
            user_api: &User,
            symbol: "price".into(),
            buying,
            ctx: context(10_000),
        }
    }
    fn risk() -> risk_reversal::RiskReversal<'static> {
        risk_reversal::RiskReversal {
            amount: Amount::from_sat(6000),
            current_price_x_one_unit: 30_000,
            range: ((1, 3), (1, 3)),
            operator_api: &Operator,
            user_api: &User,
            symbol: "price".into(),
            ctx: context(9000),
        }
    }
    fn payouts(args: &GenericBetArguments<'_>) -> Vec<(i64, u64, u64)> {
        args.outcomes
            .iter()
            .map(|(price, template)| {
                assert!(!template.tx.output.is_empty());
                assert!(template
                    .tx
                    .output
                    .iter()
                    .all(|output| output.value.to_sat() > 0));
                let received = |recipient: bitcoin::Address| {
                    template
                        .tx
                        .output
                        .iter()
                        .filter(|output| output.script_pubkey == recipient.script_pubkey())
                        .map(|output| output.value.to_sat())
                        .sum::<u64>()
                };
                let user = received(address(2));
                let operator = received(address(1));
                assert_eq!(user + operator, args.amount.to_sat());
                (*price, user, operator)
            })
            .collect()
    }
    #[test]
    fn long_and_short_calls_puts_apply_notional_and_price_scale() {
        let long = GenericBetArguments::try_from(call(true, 30_000, 10_000)).unwrap();
        assert_eq!(long.amount.to_sat(), 10_000);
        assert_eq!(
            payouts(&long),
            vec![
                (10_000, 0, 10_000),
                (20_000, 5000, 5000),
                (30_000, 10_000, 0)
            ]
        );
        let short = GenericBetArguments::try_from(call(false, 30_000, 10_000)).unwrap();
        assert_eq!(
            payouts(&short),
            vec![
                (10_000, 10_000, 0),
                (20_000, 5000, 5000),
                (30_000, 0, 10_000)
            ]
        );
        let long = GenericBetArguments::try_from(put(true)).unwrap();
        assert_eq!(
            payouts(&long),
            vec![(0, 10_000, 0), (10_000, 5000, 5000), (20_000, 0, 10_000)]
        );
        let short = GenericBetArguments::try_from(put(false)).unwrap();
        assert_eq!(
            payouts(&short),
            vec![(0, 0, 10_000), (10_000, 5000, 5000), (20_000, 10_000, 0)]
        );
        let off_grid = GenericBetArguments::try_from(call(true, 25_000, 7500)).unwrap();
        assert_eq!(
            payouts(&off_grid),
            vec![(10_000, 0, 7500), (20_000, 5000, 2500), (25_000, 7500, 0)]
        );
        assert!(GenericBet::try_from(off_grid)
            .unwrap()
            .compile(context(7500))
            .is_ok());
        assert!(GenericBet::try_from(put(true))
            .unwrap()
            .compile(context(10_000))
            .is_ok());
    }
    #[test]
    fn risk_reversal_preserves_purchasing_power_and_collateral() {
        let args = GenericBetArguments::try_from(risk()).unwrap();
        assert_eq!(args.amount.to_sat(), 9000);
        assert_eq!(
            payouts(&args),
            vec![
                (20_000, 9000, 0),
                (30_000, 6000, 3000),
                (40_000, 4500, 4500)
            ]
        );
        let compiled = GenericBet::try_from(args)
            .unwrap()
            .compile(context(9000))
            .unwrap();
        assert_eq!(compiled.ctv_to_tx.len(), 2);
        let descriptor = serde_json::to_string(&compiled.descriptor).unwrap();
        assert!(descriptor.contains(&key(1).to_string()));
        assert!(descriptor.contains(&key(2).to_string()));
    }
    #[test]
    fn invalid_price_arithmetic_is_rejected_before_iteration() {
        let mut c = call(true, 30_000, 10_000);
        c.amount = Amount::from_sat(0);
        assert!(GenericBetArguments::try_from(c).is_err());
        assert!(GenericBetArguments::try_from(call(true, 9999, 10_000)).is_err());
        assert!(GenericBetArguments::try_from(call(true, 10_000, 10_000)).is_err());
        assert!(GenericBetArguments::try_from(call(true, u64::MAX, 10_000)).is_err());
        let mut c = call(true, 30_000, 10_000);
        c.amount = Amount::from_sat(u64::MAX);
        assert!(GenericBetArguments::try_from(c).is_err());
        let mut p = put(true);
        p.strike_x_one_unit = 0;
        assert!(GenericBetArguments::try_from(p).is_err());
        for range in [
            ((1, 0), (1, 3)),
            ((1, 3), (1, 0)),
            ((1, 1), (1, 3)),
            ((2, 1), (1, 3)),
            ((1, 3), (u64::MAX, 1)),
        ] {
            let mut r = risk();
            r.range = range;
            assert!(GenericBetArguments::try_from(r).is_err());
        }
        let mut r = risk();
        r.current_price_x_one_unit = 1;
        assert!(GenericBetArguments::try_from(r).is_err());
        let mut r = risk();
        r.amount = Amount::from_sat(u64::MAX);
        assert!(GenericBetArguments::try_from(r).is_err());
        assert_eq!(
            scaled_amount(Amount::from_sat(u64::MAX), 2, 2)
                .unwrap()
                .to_sat(),
            u64::MAX
        );
    }
    #[test]
    fn generic_bet_rejects_ambiguous_outcomes_and_compiles_singleton() {
        let mut args = GenericBetArguments::try_from(put(true)).unwrap();
        args.outcomes.clear();
        assert!(GenericBet::try_from(args).is_err());
        let mut args = GenericBetArguments::try_from(put(true)).unwrap();
        args.outcomes[1].0 = args.outcomes[0].0;
        assert!(GenericBet::try_from(args).is_err());
        let mut args = GenericBetArguments::try_from(put(true)).unwrap();
        args.amount = Amount::from_sat(9999);
        assert!(GenericBet::try_from(args).is_err());
        let mut args = GenericBetArguments::try_from(put(true)).unwrap();
        args.outcomes.truncate(1);
        let bet = GenericBet::try_from(args).unwrap();
        let compiled = bet.compile(context(10_000)).unwrap();
        assert_eq!(compiled.ctv_to_tx.len(), 1);
        assert!(bet.compile(context(9999)).is_err());
        assert_eq!(
            compiled.ctv_to_tx.values().next().unwrap().tx.output[0]
                .value
                .to_sat(),
            10_000
        );
    }
    #[test]
    fn threshold_oracle_requires_a_nonempty_quorum() {
        assert!(oracle::ThresholdOracle::new(0, vec![Box::new(PriceOracle)]).is_err());
        assert!(oracle::ThresholdOracle::new(2, vec![Box::new(PriceOracle)]).is_err());
        assert!(oracle::ThresholdOracle::new(1, vec![]).is_err());
        let oracle = oracle::ThresholdOracle::new(1, vec![Box::new(PriceOracle)]).unwrap();
        assert_eq!(
            oracle.get_key_lt_gte(&"price".into(), 10_000),
            (
                Clause::Thresh(
                    Threshold::new(1, vec![Clause::Key(key(11)).into()]).expect("valid threshold")
                ),
                Clause::Thresh(
                    Threshold::new(1, vec![Clause::Key(key(21)).into()]).expect("valid threshold")
                )
            )
        );
    }
}
