// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Put Contract
use super::*;
/// Put Contracts pay out as the price goes down.
pub struct Put<'a> {
    /// The # of units
    amount: Amount,
    /// The strike with ONE_UNIT precision (bitcoin per symbol)
    strike_x_one_unit: u64,
    operator_api: &'a dyn apis::OperatorApi,
    user_api: &'a dyn apis::UserApi,
    symbol: Symbol,
    /// whether we are buying or selling the put
    buying: bool,
    ctx: Context,
}

const ONE_UNIT: u64 = 10_000;
impl<'a> TryFrom<Put<'a>> for GenericBetArguments<'a> {
    type Error = CompilationError;
    fn try_from(v: Put<'a>) -> Result<Self, Self::Error> {
        let key = v.operator_api.get_key();
        let user = v.user_api.get_key();
        let mut outcomes = vec![];
        let strike = v.strike_x_one_unit;
        let max_amount_bitcoin = v.amount * strike;
        // Increment 1 dollar per step
        for price in (0..=strike).step_by(ONE_UNIT as usize) {
            let mut profit = Amount::from_sat(strike) - Amount::from_sat(price);
            let mut refund = max_amount_bitcoin - profit;
            if v.buying {
                std::mem::swap(&mut profit, &mut refund);
            }
            outcomes.push((
                price as i64,
                v.ctx
                    .template()
                    .add_output(profit, &v.user_api.receive_payment(profit), None)?
                    .add_output(refund, &v.operator_api.receive_payment(refund), None)?
                    .into(),
            ));
        }
        // Now that the schedule is constructed, build a contract
        Ok(GenericBetArguments {
            // must send max amount for the contract to be valid!
            amount: max_amount_bitcoin,
            outcomes,
            oracle: v.operator_api.get_oracle(),
            cooperate: Clause::And(vec![key, user]),
            symbol: v.symbol,
        })
    }
}

impl<'a> TryFrom<Put<'a>> for GenericBet {
    type Error = CompilationError;
    fn try_from(v: Put<'a>) -> Result<Self, Self::Error> {
        Ok(GenericBetArguments::try_from(v)?.into())
    }
}
