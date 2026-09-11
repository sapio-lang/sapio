// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Put Contract
use super::*;
use std::sync::Arc;
/// Put Contracts pay out as the price goes down.
pub struct Put<'a> {
    /// Satoshi notional paid per whole oracle-price unit (PRICE_UNIT).
    pub amount: Amount,
    /// The strike with PRICE_UNIT precision
    pub strike_x_one_unit: u64,
    /// Operator keys, oracle, and payout destination.
    pub operator_api: &'a dyn apis::OperatorApi,
    /// Counterparty key and payout destination.
    pub user_api: &'a dyn apis::UserApi,
    /// Oracle price symbol.
    pub symbol: Symbol,
    /// whether we are buying or selling the put
    pub buying: bool,
    /// Compilation context containing the full collateral.
    pub ctx: Context,
}

impl<'a> TryFrom<Put<'a>> for GenericBetArguments<'a> {
    type Error = CompilationError;
    fn try_from(mut v: Put<'a>) -> Result<Self, Self::Error> {
        let key = v.operator_api.get_key();
        let user = v.user_api.get_key();
        let mut outcomes = vec![];
        let strike = v.strike_x_one_unit;
        if v.amount.to_sat() == 0 || strike == 0 {
            return Err(invalid("Invalid put notional or strike"));
        }
        let max_amount_bitcoin = scaled_amount(v.amount, strike, PRICE_UNIT)?;
        if max_amount_bitcoin.to_sat() == 0 {
            return Err(invalid("Option collateral rounds to zero"));
        }
        // Increment one whole oracle-price unit per step
        let mut strike_ctx = v.ctx.derive_str(Arc::new("strike".into()))?;
        for price in price_grid(0, strike)? {
            let mut profit = scaled_amount(v.amount, strike - price, PRICE_UNIT)?;
            let mut refund = max_amount_bitcoin - profit;
            if !v.buying {
                std::mem::swap(&mut profit, &mut refund);
            }
            outcomes.push((
                price as i64,
                settlement(
                    strike_ctx.derive_num(price)?,
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
            cooperate: Clause::And(vec![key.into(), user.into()]),
            symbol: v.symbol,
        })
    }
}

impl<'a> TryFrom<Put<'a>> for GenericBet {
    type Error = CompilationError;
    fn try_from(v: Put<'a>) -> Result<Self, Self::Error> {
        GenericBetArguments::try_from(v)?.try_into()
    }
}
