// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Call Options
use super::*;
/// A Call Option -- the buyer gains money as the price increases.
pub struct Call<'a> {
    /// The # of units
    amount: Amount,
    /// The strike with ONE_UNIT precision (bitcoin per symbol)
    strike_x_one_unit: u64,
    /// The max price with ONE_UNIT precision (bitcoin per symbol)
    /// Because these are fully collateralized contracts, we can't do an
    /// actual call.
    max_price_x_one_unit: u64,
    operator_api: &'a dyn apis::OperatorApi,
    user_api: &'a dyn apis::UserApi,
    symbol: Symbol,
    /// whether we are buying or selling the Call
    buying: bool,
    ctx: Context,
}

const ONE_UNIT: u64 = 10_000;
impl<'a> TryFrom<Call<'a>> for GenericBetArguments<'a> {
    type Error = CompilationError;
    fn try_from(v: Call<'a>) -> Result<Self, Self::Error> {
        let key = v.operator_api.get_key();
        let user = v.user_api.get_key();
        let mut outcomes = vec![];
        let strike = v.strike_x_one_unit;
        let max_amount_bitcoin = v.amount * v.max_price_x_one_unit;
        // Increment 1 dollar per step
        for price in (strike..=v.max_price_x_one_unit).step_by(ONE_UNIT as usize) {
            let mut profit = Amount::from_sat(price) - Amount::from_sat(strike);
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

impl<'a> TryFrom<Call<'a>> for GenericBet {
    type Error = CompilationError;
    fn try_from(v: Call<'a>) -> Result<Self, Self::Error> {
        Ok(GenericBetArguments::try_from(v)?.into())
    }
}
