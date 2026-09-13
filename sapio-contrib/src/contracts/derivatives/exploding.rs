// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Contracts which have a expiration date before which they must be executed...
use super::*;
use sapio_base::timelocks::*;
use sapio_macros::guard;
/// Generic functionality required for Exploding contracts
pub trait Explodes: 'static + Sized + Contract {
    decl_then! {
        /// What to do when the timeout expires
        explodes
    }
    decl_then! {
        /// what to do when the holder wishes to strike
        strikes
    }
}

impl<T> Contract for ExplodingOption<T>
where
    GenericBet: TryFrom<T>,
    CompilationError: From<<GenericBet as TryFrom<T>>::Error>,
    T: Clone + 'static,
{
    fn ensure_amount(&self, _ctx: Context) -> Result<Amount, CompilationError> {
        checked_collateral(self.party_one, self.party_two)
    }
    declare! {actions, Self::explodes, Self::strikes}
}

impl<T> Contract for UnderFundedExplodingOption<T>
where
    GenericBet: TryFrom<T>,
    CompilationError: From<<GenericBet as TryFrom<T>>::Error>,
    T: Clone + 'static,
{
    fn ensure_amount(&self, _ctx: Context) -> Result<Amount, CompilationError> {
        checked_collateral(self.party_one, self.party_two)?;
        Ok(self.party_one)
    }
    declare! {actions, Self::explodes, Self::strikes}
}
/// Wraps an option with a refund for both parties on timeout.
/// Convert Call/Put/RiskReversal arguments into a GenericBet first; its immutable
/// settlement schedule can be cloned without cloning a compilation Context.
/// Custom Clone arguments with fallible conversions remain supported.
pub struct ExplodingOption<T: 'static> {
    /// First party collateral.
    pub party_one: Amount,
    /// Second party collateral.
    pub party_two: Amount,
    /// First party refund address.
    pub key_p1: bitcoin::Address,
    /// Second party refund address.
    pub key_p2: bitcoin::Address,
    /// Second party authorization to exercise.
    pub key_p2_pk: Clause,
    /// Option converted into its settlement schedule.
    pub opt: T,
    /// Absolute refund timeout.
    pub timeout: AnyAbsTimeLock,
}

impl<T> ExplodingOption<T> {
    #[guard]
    fn signed(self, _ctx: Context) {
        self.key_p2_pk.clone()
    }
}
impl<T> Explodes for ExplodingOption<T>
where
    GenericBet: TryFrom<T>,
    CompilationError: From<<GenericBet as TryFrom<T>>::Error>,
    T: Clone,
{
    #[then]
    fn explodes(self, ctx: sapio::Context) {
        ctx.template()
            .add_output(
                self.party_one,
                &Compiled::from_address(self.key_p1.clone(), bitcoin::Amount::ZERO),
                None,
            )?
            .add_output(
                self.party_two,
                &Compiled::from_address(self.key_p2.clone(), bitcoin::Amount::ZERO),
                None,
            )?
            .set_lock_time(self.timeout)?
            .into()
    }

    #[then(guarded_by = "[Self::signed]")]
    fn strikes(self, ctx: sapio::Context) {
        let option = GenericBet::try_from(self.opt.clone())?;
        if option.amount != checked_collateral(self.party_one, self.party_two)? {
            return Err(invalid(
                "Option collateral differs from the deposited amounts",
            ));
        }
        ctx.template()
            .add_output(option.amount, &option, None)?
            .into()
    }
}

/// Similar to `ExplodingOption` except that the option requires an additional
/// value amount to be paid in in order to execute, hence being "under funded"
pub struct UnderFundedExplodingOption<T: 'static> {
    /// First party collateral.
    pub party_one: Amount,
    /// Second party collateral.
    pub party_two: Amount,
    /// First party refund address.
    pub key_p1: bitcoin::Address,
    /// Option converted into its settlement schedule.
    pub opt: T,
    /// Absolute refund timeout.
    pub timeout: AnyAbsTimeLock,
}

impl<T> Explodes for UnderFundedExplodingOption<T>
where
    GenericBet: TryFrom<T>,
    CompilationError: From<<GenericBet as TryFrom<T>>::Error>,
    T: Clone,
{
    #[then]
    fn explodes(self, ctx: sapio::Context) {
        ctx.template()
            .add_output(
                self.party_one,
                &Compiled::from_address(self.key_p1.clone(), bitcoin::Amount::ZERO),
                None,
            )?
            .set_lock_time(self.timeout)?
            .into()
    }

    #[then]
    fn strikes(self, ctx: sapio::Context) {
        let option = GenericBet::try_from(self.opt.clone())?;
        if option.amount != checked_collateral(self.party_one, self.party_two)? {
            return Err(invalid(
                "Option collateral differs from the deposited amounts",
            ));
        }
        ctx.template()
            .add_sequence()
            .add_amount(self.party_two)?
            .add_output(
                checked_collateral(self.party_one, self.party_two)?,
                &option,
                None,
            )?
            .into()
    }
}

fn checked_collateral(first: Amount, second: Amount) -> Result<Amount, CompilationError> {
    first
        .checked_add(second)
        .filter(|amount| amount.to_sat() > 0)
        .ok_or_else(|| invalid("Exploding option collateral overflows or is empty"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{address, context, key};
    #[derive(Clone)]
    struct OptionFixture(u64);
    struct PriceOracle;
    impl Oracle for PriceOracle {
        fn get_key_lt_gte(&self, _: &Symbol, _: i64) -> (Clause, Clause) {
            (Clause::Key(key(3)), Clause::Key(key(4)))
        }
    }
    impl TryFrom<OptionFixture> for GenericBet {
        type Error = CompilationError;
        fn try_from(fixture: OptionFixture) -> Result<Self, Self::Error> {
            let amount = Amount::from_sat(fixture.0);
            let template = context(fixture.0)
                .template()
                .add_output(amount, &key(1), None)?
                .into();
            GenericBetArguments {
                amount,
                outcomes: vec![(0, template)],
                oracle: &PriceOracle,
                cooperate: Clause::And(vec![
                    Clause::Key(key(1)).into(),
                    Clause::Key(key(2)).into(),
                ]),
                symbol: "price".into(),
            }
            .try_into()
        }
    }
    fn funded() -> ExplodingOption<OptionFixture> {
        ExplodingOption {
            party_one: Amount::from_sat(1000),
            party_two: Amount::from_sat(2000),
            key_p1: address(1),
            key_p2: address(2),
            key_p2_pk: Clause::Key(key(2)),
            opt: OptionFixture(3000),
            timeout: AbsHeight::try_from(100).unwrap().into(),
        }
    }
    fn underfunded() -> UnderFundedExplodingOption<OptionFixture> {
        UnderFundedExplodingOption {
            party_one: Amount::from_sat(1000),
            party_two: Amount::from_sat(2000),
            key_p1: address(1),
            opt: OptionFixture(3000),
            timeout: AbsHeight::try_from(100).unwrap().into(),
        }
    }
    #[test]
    fn prepared_bets_compose_without_cloning_a_context() {
        let prepared = GenericBet::try_from(OptionFixture(3000)).unwrap();
        let expected = bitcoin::ScriptBuf::from(&prepared.compile(context(3000)).unwrap().address);
        let funded = ExplodingOption {
            party_one: Amount::from_sat(1000),
            party_two: Amount::from_sat(2000),
            key_p1: address(1),
            key_p2: address(2),
            key_p2_pk: Clause::Key(key(2)),
            opt: prepared.clone(),
            timeout: AbsHeight::try_from(100).unwrap().into(),
        };
        let underfunded = UnderFundedExplodingOption {
            party_one: Amount::from_sat(1000),
            party_two: Amount::from_sat(2000),
            key_p1: address(1),
            opt: prepared,
            timeout: AbsHeight::try_from(100).unwrap().into(),
        };
        for compiled in [
            funded.compile(context(3000)).unwrap(),
            underfunded.compile(context(1000)).unwrap(),
        ] {
            let exercise = compiled
                .ctv_to_tx
                .values()
                .find(|t| t.tx.lock_time.to_consensus_u32() == 0)
                .unwrap();
            assert_eq!(exercise.tx.output[0].script_pubkey, expected);
            assert_eq!(exercise.tx.output[0].value.to_sat(), 3000);
        }
    }

    #[test]
    fn funded_refund_and_strike_conserve_collateral() {
        let compiled = funded().compile(context(3000)).unwrap();
        assert_eq!(compiled.ctv_to_tx.len(), 2);
        let refund = compiled
            .ctv_to_tx
            .values()
            .find(|t| t.tx.lock_time.to_consensus_u32() == 100)
            .unwrap();
        assert_eq!(
            refund
                .tx
                .output
                .iter()
                .map(|o| o.value.to_sat())
                .collect::<Vec<_>>(),
            vec![1000, 2000]
        );
        assert_eq!(
            refund.tx.output[0].script_pubkey,
            address(1).script_pubkey()
        );
        assert_eq!(
            refund.tx.output[1].script_pubkey,
            address(2).script_pubkey()
        );
        let strike = compiled
            .ctv_to_tx
            .values()
            .find(|t| t.tx.lock_time.to_consensus_u32() == 0)
            .unwrap();
        assert_eq!(strike.tx.input.len(), 1);
        assert_eq!(strike.tx.output[0].value.to_sat(), 3000);
        assert_eq!(strike.outputs[0].contract.ctv_to_tx.len(), 1);
    }
    #[test]
    fn underfunded_exercise_requires_an_additional_input() {
        let compiled = underfunded().compile(context(1000)).unwrap();
        assert_eq!(compiled.required_input_amount.to_sat(), 1000);
        let strike = compiled
            .ctv_to_tx
            .values()
            .find(|t| t.tx.input.len() == 2)
            .unwrap();
        assert_eq!(strike.tx.output[0].value.to_sat(), 3000);
        assert_eq!(strike.required_input_amount.to_sat(), 1000);
        assert_eq!(strike.max.to_sat(), 3000);
        let refund = compiled
            .ctv_to_tx
            .values()
            .find(|t| t.tx.input.len() == 1)
            .unwrap();
        assert_eq!(refund.tx.output[0].value.to_sat(), 1000);
        assert_eq!(refund.tx.lock_time.to_consensus_u32(), 100);
        let mut bad = funded();
        bad.opt = OptionFixture(2000);
        assert!(bad.compile(context(3000)).is_err());
        let mut bad = funded();
        bad.party_two = Amount::from_sat(u64::MAX);
        assert!(bad.compile(context(3000)).is_err());
        let mut bad = underfunded();
        bad.opt = OptionFixture(2000);
        assert!(bad.compile(context(1000)).is_err());
    }
}
