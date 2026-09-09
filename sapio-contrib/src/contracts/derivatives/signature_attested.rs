// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Payout schedules authorized by explicit oracle transaction signatures.
//!
//! This is not an adaptor-signature DLC protocol. Each settlement requires the
//! configured oracle threshold to sign its Bitcoin transaction. Oracle outcome
//! selection and key authentication happen outside this example; no public-key
//! arithmetic substitutes for those signatures.
use super::invalid;
use bitcoin::{Amount, XOnlyPublicKey};
use sapio::contract::*;
use sapio::template::Template;
use sapio::*;
use sapio_base::Clause;
use std::collections::BTreeSet;

/// Supplies an authenticated signing key for an event outcome.
pub trait OutcomeOracle {
    /// The oracle signs the settlement transaction only when this outcome holds.
    fn outcome_key(&self, event: &str, outcome: u32) -> Result<XOnlyPublicKey, CompilationError>;
}

/// Nonnegative integer payout weights, one per party. The sum must be positive.
pub type Curve = Box<dyn Fn(u32, usize) -> Result<Vec<u64>, CompilationError>>;

/// A finite collection of oracle-authorized settlement transactions.
pub struct SignatureAttested {
    /// Number of agreeing oracles, and the participating oracle implementations.
    pub oracles: (usize, Vec<Box<dyn OutcomeOracle>>),
    /// Integer payout weights at each outcome.
    pub curve: Curve,
    /// Highest outcome index; settlements include both zero and this index.
    pub points: u32,
    /// Distinct recipient/cooperative signing keys (at least two).
    pub parties: Vec<XOnlyPublicKey>,
    /// Event identifier understood by every oracle.
    pub event: String,
}

fn allocate(funds: Amount, weights: &[u64]) -> Result<Vec<Amount>, CompilationError> {
    let total = weights
        .iter()
        .try_fold(0u64, |sum, weight| sum.checked_add(*weight))
        .filter(|total| *total > 0)
        .ok_or_else(|| invalid("Invalid payout weight total"))?;
    let mut cumulative = 0u64;
    let mut paid = 0u64;
    Ok(weights
        .iter()
        .map(|weight| {
            cumulative += weight;
            let through_here =
                (u128::from(funds.as_sat()) * u128::from(cumulative) / u128::from(total)) as u64;
            let amount = Amount::from_sat(through_here - paid);
            paid = through_here;
            amount
        })
        .collect())
}
impl SignatureAttested {
    #[guard]
    fn cooperate(self, _ctx: Context) {
        Clause::And(self.parties.iter().copied().map(Clause::Key).collect())
    }
    #[then]
    fn payout(self, mut ctx: Context) {
        let funds = ctx.funds();
        let mut settlements = vec![];
        for point in 0..=self.points {
            let keys = self
                .oracles
                .1
                .iter()
                .map(|oracle| oracle.outcome_key(&self.event, point))
                .collect::<Result<Vec<_>, _>>()?;
            if keys.iter().copied().collect::<BTreeSet<_>>().len() != keys.len() {
                return Err(invalid(
                    "Duplicate oracle keys cannot count toward the threshold",
                ));
            }
            let weights = (self.curve)(point, self.parties.len())?;
            if weights.len() != self.parties.len() {
                return Err(invalid("Payout count must match the parties"));
            }
            let payouts = allocate(funds, &weights)?;
            let guard =
                Clause::Threshold(self.oracles.0, keys.into_iter().map(Clause::Key).collect());
            let mut builder = ctx.derive_num(point)?.template().add_guard(guard);
            for (party, amount) in self.parties.iter().zip(payouts) {
                if amount.as_sat() > 0 {
                    builder = builder.add_output(amount, party, None)?;
                }
            }
            let template: Template = builder.into();
            settlements.push(Ok(template));
        }
        Ok(Box::new(settlements.into_iter()))
    }
}
impl Contract for SignatureAttested {
    fn ensure_amount(&self, ctx: Context) -> Result<Amount, CompilationError> {
        if self.parties.len() < 2
            || self.parties.iter().collect::<BTreeSet<_>>().len() != self.parties.len()
            || self.oracles.0 == 0
            || self.oracles.0 > self.oracles.1.len()
            || ctx.funds().as_sat() == 0
        {
            return Err(invalid("Invalid parties, oracle threshold, or collateral"));
        }
        Ok(ctx.funds())
    }
    declare! {then, Self::payout}
    declare! {finish, Self::cooperate}
    declare! {non updatable}
}

/// Two-party example curves, quantized to billionth-share integer weights.
/// Monetary allocation uses integer arithmetic after this explicit quantization.
#[derive(Clone, Copy)]
pub enum SplitFunctions {
    /// Linear growth from the initial fraction to one.
    LinearPositive(f64),
    /// Geometric growth from a strictly positive initial fraction to one.
    GeometricPositive(f64),
    /// Logistic curve centered at this outcome index.
    Sigmoid(f64),
}
impl SplitFunctions {
    /// Create a two-party curve over a positive number of intervals.
    pub fn get_curve(self, points: u32) -> Result<Curve, CompilationError> {
        let valid = match self {
            Self::LinearPositive(b) => b.is_finite() && (0.0..=1.0).contains(&b),
            Self::GeometricPositive(b) => b.is_finite() && b > 0.0 && b <= 1.0,
            Self::Sigmoid(offset) => offset.is_finite(),
        };
        if points == 0 || !valid {
            return Err(invalid("Invalid payout curve parameters"));
        }
        Ok(Box::new(move |point, parties| {
            if parties != 2 || point > points {
                return Err(invalid("Invalid payout curve point or party count"));
            }
            let fraction = match self {
                Self::LinearPositive(b) => b + (1.0 - b) * f64::from(point) / f64::from(points),
                Self::GeometricPositive(b) => {
                    if point == points {
                        1.0
                    } else {
                        b.powf(1.0 - f64::from(point) / f64::from(points))
                    }
                }
                Self::Sigmoid(offset) => 1.0 / (1.0 + (offset - f64::from(point)).exp()),
            };
            if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
                return Err(invalid("Invalid payout curve result"));
            }
            const SCALE: u64 = 1_000_000_000;
            let weight = (fraction * SCALE as f64).round() as u64;
            Ok(vec![weight, SCALE - weight])
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{context, key};
    struct Oracle(u8);
    impl OutcomeOracle for Oracle {
        fn outcome_key(
            &self,
            event: &str,
            outcome: u32,
        ) -> Result<XOnlyPublicKey, CompilationError> {
            assert_eq!(event, "price");
            Ok(key(self.0 + outcome as u8))
        }
    }
    fn contract() -> SignatureAttested {
        SignatureAttested {
            oracles: (2, vec![Box::new(Oracle(10)), Box::new(Oracle(20))]),
            curve: SplitFunctions::LinearPositive(0.0).get_curve(2).unwrap(),
            points: 2,
            parties: vec![key(1), key(2)],
            event: "price".into(),
        }
    }
    #[test]
    fn exact_settlements_require_the_declared_outcome_keys() {
        let compiled = contract().compile(context(1001)).unwrap();
        assert_eq!(compiled.ctv_to_tx.len(), 3);
        let mut distributions = vec![];
        for template in compiled.ctv_to_tx.values() {
            assert_eq!(
                template.tx.output.iter().map(|o| o.value).sum::<u64>(),
                1001
            );
            let point = if template.tx.output.len() == 2 {
                1
            } else {
                let recipient = key(1).compile(context(1001)).unwrap();
                if template.tx.output[0].script_pubkey == bitcoin::Script::from(&recipient.address)
                {
                    2
                } else {
                    0
                }
            };
            assert_eq!(
                template.guards,
                vec![Clause::Threshold(
                    2,
                    vec![Clause::Key(key(10 + point)), Clause::Key(key(20 + point))]
                )
                .into()]
            );
            distributions.push(
                template
                    .tx
                    .output
                    .iter()
                    .map(|o| o.value)
                    .collect::<Vec<_>>(),
            );
        }
        distributions.sort();
        assert_eq!(distributions, vec![vec![500, 501], vec![1001], vec![1001]]);
        assert_eq!(
            allocate(Amount::from_sat(u64::MAX), &[1, 1]).unwrap(),
            vec![
                Amount::from_sat(u64::MAX / 2),
                Amount::from_sat(u64::MAX / 2 + 1)
            ]
        );
        assert_eq!(
            allocate(Amount::from_sat(10), &[1, 1, 1])
                .unwrap()
                .iter()
                .map(|amount| amount.as_sat())
                .collect::<Vec<_>>(),
            vec![3, 3, 4]
        );
    }
    #[test]
    fn equal_payouts_retain_all_outcome_authorizations() {
        let mut c = contract();
        c.curve = Box::new(|_, _| Ok(vec![1, 1]));
        let compiled = c.compile(context(1001)).unwrap();
        assert_eq!(compiled.ctv_to_tx.len(), 1);
        let template = compiled.ctv_to_tx.values().next().unwrap();
        assert_eq!(
            template
                .tx
                .output
                .iter()
                .map(|o| o.value)
                .collect::<Vec<_>>(),
            vec![500, 501]
        );
        let expected = Clause::Threshold(
            1,
            (0..=2)
                .map(|point| {
                    Clause::Threshold(
                        2,
                        vec![Clause::Key(key(10 + point)), Clause::Key(key(20 + point))],
                    )
                })
                .collect(),
        );
        use sapio_base::miniscript::policy::Liftable;
        assert_eq!(template.guards.len(), 1);
        let sapio_base::policy::ScriptPolicy::Miniscript(actual) = &template.guards[0] else {
            panic!()
        };
        let actual = actual.lift().unwrap();
        let expected = expected.lift().unwrap();
        assert!(actual.clone().entails(expected.clone()).unwrap());
        assert!(expected.entails(actual).unwrap());
        let descriptor = serde_json::to_string(&compiled.descriptor).unwrap();
        for point in 0..=2 {
            for oracle in [10, 20] {
                assert!(descriptor.contains(&key(oracle + point).to_string()));
            }
        }
    }

    #[test]
    fn curve_endpoints_sigmoid_and_invalid_parameters() {
        for curve in [
            SplitFunctions::LinearPositive(0.25),
            SplitFunctions::GeometricPositive(0.25),
        ] {
            let curve = curve.get_curve(2).unwrap();
            assert_eq!(curve(0, 2).unwrap(), vec![250_000_000, 750_000_000]);
            assert_eq!(curve(2, 2).unwrap(), vec![1_000_000_000, 0]);
            assert!(curve(3, 2).is_err());
            assert!(curve(1, 3).is_err());
        }
        let sigmoid = SplitFunctions::Sigmoid(1.0).get_curve(2).unwrap();
        assert_eq!(sigmoid(1, 2).unwrap(), vec![500_000_000, 500_000_000]);
        assert!(sigmoid(0, 2).unwrap()[0] < 500_000_000);
        assert!(sigmoid(2, 2).unwrap()[0] > 500_000_000);
        for curve in [
            SplitFunctions::LinearPositive(-0.1),
            SplitFunctions::LinearPositive(1.1),
            SplitFunctions::GeometricPositive(0.0),
            SplitFunctions::Sigmoid(f64::NAN),
        ] {
            assert!(curve.get_curve(2).is_err());
        }
        assert!(SplitFunctions::LinearPositive(0.0).get_curve(0).is_err());
    }
    #[test]
    fn invalid_threshold_parties_weights_and_duplicate_oracles_fail() {
        let mut c = contract();
        c.oracles.0 = 0;
        assert!(c.compile(context(1000)).is_err());
        let mut c = contract();
        c.oracles.0 = 3;
        assert!(c.compile(context(1000)).is_err());
        let mut c = contract();
        c.parties[1] = c.parties[0];
        assert!(c.compile(context(1000)).is_err());
        let mut c = contract();
        c.oracles.1[1] = Box::new(Oracle(10));
        assert!(c.compile(context(1000)).is_err());
        for weights in [vec![0, 0], vec![1], vec![u64::MAX, 1]] {
            let mut c = contract();
            c.curve = Box::new(move |_, _| Ok(weights.clone()));
            assert!(c.compile(context(1000)).is_err());
        }
        assert!(contract().compile(context(0)).is_err());
    }
}
