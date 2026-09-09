// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! RiskReversal represents a specific contract where we specify a set of price ranges that we
//! want to keep purchasing power flat within.
use super::*;
use std::sync::Arc;
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
    /// Bitcoin notional whose purchasing power is maintained.
    pub amount: Amount,
    /// the current price in dollars with one_unit precision
    pub current_price_x_one_unit: u64,
    /// price multipliers rationals (lo, hi) and (a,b) = a/b
    /// e.g. ((7, 91), (1, 10)) computes from price - price*7/91 to price + price*1/10
    pub range: ((u64, u64), (u64, u64)),
    /// Operator keys, oracle, and payout destination.
    pub operator_api: &'a dyn apis::OperatorApi,
    /// Counterparty key and payout destination.
    pub user_api: &'a dyn apis::UserApi,
    /// Oracle price symbol.
    pub symbol: Symbol,
    /// Compilation context containing the full collateral.
    pub ctx: Context,
}

impl<'a> TryFrom<RiskReversal<'a>> for GenericBetArguments<'a> {
    type Error = CompilationError;
    fn try_from(mut v: RiskReversal<'a>) -> Result<Self, Self::Error> {
        let key = v.operator_api.get_key();
        let user = v.user_api.get_key();
        let mut outcomes = vec![];
        let current_price = v.current_price_x_one_unit;
        let ((down, down_den), (up, up_den)) = v.range;
        if current_price == 0
            || v.amount.as_sat() == 0
            || down_den == 0
            || up_den == 0
            || down >= down_den
        {
            return Err(invalid("Invalid risk reversal notional, price, or range"));
        }
        let current = u128::from(current_price);
        let bottom = ((current - current * u128::from(down) / u128::from(down_den))
            / u128::from(PRICE_UNIT))
            * u128::from(PRICE_UNIT);
        let top = (current + current * u128::from(up) / u128::from(up_den))
            .div_ceil(u128::from(PRICE_UNIT))
            * u128::from(PRICE_UNIT);
        let bottom = u64::try_from(bottom).map_err(|_| invalid("Risk reversal price overflow"))?;
        let top = u64::try_from(top).map_err(|_| invalid("Risk reversal price overflow"))?;
        if bottom == 0 {
            return Err(invalid("Risk reversal lower price rounds to zero"));
        }
        let max_amount_bitcoin = scaled_amount(v.amount, current_price, bottom)?;

        let mut strike_ctx = v.ctx.derive_str(Arc::new("strike".into()))?;
        // Increment 1 dollar per step
        for strike in price_grid(bottom, top)? {
            // Value Conservation Property:
            // strike * (amount + delta)  == amount * current price
            // strike * (pay to user)  == amount * current price
            // pay to user  == amount * current price / strike
            let profit = scaled_amount(v.amount, current_price, strike)?;
            let refund = max_amount_bitcoin - profit;

            outcomes.push((
                strike as i64,
                settlement(
                    strike_ctx.derive_num(strike)?,
                    profit,
                    refund,
                    v.user_api,
                    v.operator_api,
                )?,
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
        GenericBetArguments::try_from(v)?.try_into()
    }
}
