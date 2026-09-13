// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! a chain of op_returns
use bitcoin::Amount;
use sapio::contract::*;
use sapio::util::amountrange::AmountF64;
use sapio::*;
use sapio_base::Clause;
use sapio_macros::guard;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::*;

/// Chain of OpReturns
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ChainReturn {
    pk: bitcoin::XOnlyPublicKey,
}

impl ChainReturn {
    /// everyone has signed off on the transaction
    #[guard]
    fn approved(self, _ctx: Context) {
        Clause::Key(self.pk)
    }
    fn default_close(&self, ctx: Context) -> TxTmplIt {
        self.continue_next_chain(ctx, UpdateTypes::Close)
    }

    /// move the coins to the next state -- payouts may recursively contain pools itself
    #[continuation(
        guarded_by = "[Self::approved]",
        defaults = "Self::default_close",
        web_api
    )]
    fn next_chain(self, ctx: sapio::Context, o: UpdateTypes) {
        let mut tmpl = ctx.template();
        let mut pay_fees = Amount::ZERO;
        if let UpdateTypes::AddData { data, fees } = o {
            pay_fees = fees.into();
            tmpl = tmpl.add_output(
                Amount::from_sat(0),
                &Compiled::from_op_return(data.as_str().as_bytes())?,
                None,
            )?;
            let funds = tmpl
                .ctx()
                .funds()
                .checked_sub(pay_fees)
                .ok_or(CompilationError::OutOfFunds)?;
            if funds.to_sat() != 0 {
                tmpl = tmpl.add_output(funds, self, None)?;
            }
        } else {
            let funds = tmpl.ctx().funds();
            tmpl = tmpl.add_output(funds, &self.pk, None)?;
        }

        tmpl.add_fees(pay_fees)?.into()
    }
}

/// Updates to a ChainReturn
#[derive(Deserialize, JsonSchema)]
pub enum UpdateTypes {
    /// # Add This Data
    AddData {
        /// the op return to add
        data: String,
        /// Fees to pay
        fees: AmountF64,
    },
    /// Close the chain and return all funds to the owner.
    Close,
}
impl Contract for ChainReturn {
    declare! {actions, Self::next_chain}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{context, key};

    #[test]
    fn data_transition_reserves_fees_before_change() {
        let contract = ChainReturn { pk: key(1) };
        let template = contract
            .continue_next_chain(
                context(1000),
                UpdateTypes::AddData {
                    data: "hello".into(),
                    fees: Amount::from_sat(100).into(),
                },
            )
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(
            template
                .tx
                .output
                .iter()
                .map(|o| o.value.to_sat())
                .collect::<Vec<_>>(),
            vec![0, 900]
        );
        assert!(template.tx.output[0].script_pubkey.is_op_return());
        assert_eq!(contract.guard_approved(context(0)), Clause::Key(key(1)));
        contract.compile(context(1000)).unwrap().validate().unwrap();
    }

    #[test]
    fn oversized_data_and_fees_fail() {
        let contract = ChainReturn { pk: key(1) };
        for (data, fees) in [("x".repeat(41), 0), ("x".into(), 1001)] {
            assert!(contract
                .continue_next_chain(
                    context(1000),
                    UpdateTypes::AddData {
                        data,
                        fees: Amount::from_sat(fees).into()
                    }
                )
                .is_err());
        }
    }
}
