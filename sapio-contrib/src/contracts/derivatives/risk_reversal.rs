// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! RiskReversal represents a specific contract where we specify a set of price ranges that we
//! want to keep purchasing power flat within.
use super::*;
/// RiskReversal represents a specific contract where we specify a set of price ranges that we
/// want to keep purchasing power flat within. e.g.
///
/// ```text
///  Value of BTC in Asset
///     |            
///     |                                 /
///     |             a                  /
///     |        <------         b      /
///     |               -------------> /
///     |        ----------------------
///     |       /       ^
///     |      /        |
///     |     /        current price
///     |    /
///     --------------------------------------------------- price of BTC in Asset
/// ```
///
/// ```text
///  Amount of BTC
///     |            
///     |-------
///     |       \
///     |        \  ^
///     |         \  \
///     |          \  \
///     |           \  \
///     |            \  \  a
///     |             \  \
///     |              \  \
///     |               \  \
///     |                \  \
///     |                 \ <- current price
///     |                  \  \
///     |                   \  \
///     |                    \  \
///     |                     \  \ b
///     |                      \  \
///     |                       \  \
///     |                        \  \
///     |                         \  \
///     |                          \  \
///     |                           \  \
///     |                            \  \
///     |                             \  v
///     |                              \
///     |                               --------------
///     |    
///     --------------------------------------------------- price of BTC in Asset
/// ```
///
/// In this case, Operator would be providing enough Bitcoin (Y) for a user's funds (X) such that:
///
/// (current - a)*(X+Y) = current * X
/// or
/// Y * current = a * (X + Y)
///
/// and would be seeing a potential bitcoin gain (Z) of
///
/// (current + b) * (X - Z) = current * X
/// or
/// Z = b * X / (b + current)
///
/// or Z (current + b) dollars.
///
/// Operator can profit on the contract by:
///
/// 1. selecting carefully parameters a and b
/// 2. charging a premium
/// 3. charging a fee (& rehypothecating the position)
///
pub struct RiskReversal<'a> {
    amount: Amount,
    /// the current price in dollars with one_unit precision
    current_price_x_one_unit: u64,
    /// price multipliers rationals (lo, hi) and (a,b) = a/b
    /// e.g. ((7, 91), (1, 10)) computes from price - price*7/91 to price + price*1/10
    range: ((u64, u64), (u64, u64)),
    operator_api: &'a dyn apis::OperatorApi,
    user_api: &'a dyn apis::UserApi,
    symbol: Symbol,
    ctx: Context,
}

const ONE_UNIT: u64 = 10_000;
impl<'a> TryFrom<RiskReversal<'a>> for GenericBetArguments<'a> {
    type Error = CompilationError;
    fn try_from(v: RiskReversal<'a>) -> Result<Self, Self::Error> {
        let key = v.operator_api.get_key();
        let user = v.user_api.get_key();
        let mut outcomes = vec![];
        let current_price = v.current_price_x_one_unit;
        // TODO: Can Customize this logic to for arbitrary curves or grids
        // bottom and top are floor/ceil for where our contract operates
        let bottom =
            ((current_price - (current_price * v.range.0 .0) / v.range.0 .1) / ONE_UNIT) * ONE_UNIT;
        let top = (((current_price + (current_price * v.range.1 .0) / v.range.1 .1) + ONE_UNIT
            - 1)
            / ONE_UNIT)
            * ONE_UNIT;
        // The max amount of BTC the contract needs to meet obligations
        let max_amount_bitcoin = (v.amount * current_price) / bottom;

        // represents an overflow
        if bottom > current_price || top < current_price {
            return Err(CompilationError::TerminateCompilation);
        }

        // Increment 1 dollar per step
        for strike in (bottom..=top).step_by(ONE_UNIT as usize) {
            // Value Conservation Property:
            // strike * (amount + delta)  == amount * current price
            // strike * (pay to user)  == amount * current price
            // pay to user  == amount * current price / strike
            let profit = (v.amount * current_price) / strike;
            let refund = max_amount_bitcoin - profit;

            outcomes.push((
                strike as i64,
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

impl<'a> TryFrom<RiskReversal<'a>> for GenericBet {
    type Error = CompilationError;
    fn try_from(v: RiskReversal<'a>) -> Result<Self, Self::Error> {
        Ok(GenericBetArguments::try_from(v)?.into())
    }
}
