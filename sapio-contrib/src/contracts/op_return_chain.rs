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
use sapio_base::timelocks::AnyRelTimeLock;
use sapio_base::Clause;
use schemars::schema::RootSchema;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::*;
use std::convert::{TryFrom, TryInto};
use std::sync::{Arc, Mutex};
/// Chain of OpReturns
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ChainReturn {
    pk: bitcoin::PublicKey,
}
/// Helper
fn default_coerce(
    k: <ChainReturn as Contract>::StatefulArguments,
) -> Result<UpdateTypes, CompilationError> {
    Ok(k)
}

impl ChainReturn {
    guard! {
        /// everyone has signed off on the transaction
        fn approved(self, _ctx) {
            Clause::Key(self.pk.clone())
        }
    }
    finish! {
            /// move the coins to the next state -- payouts may recursively contain pools itself
            <web={}>
            guarded_by: [Self::approved]
            coerce_args: default_coerce
            fn next_chain(self, ctx, o: UpdateTypes) {
                let mut tmpl = ctx.template();
                if let UpdateTypes::AddData { data, fees } = o {
                    tmpl = tmpl.spend_amount(fees.into())?;
                    tmpl = tmpl.add_output(Amount::from_sat(0), &Compiled::from_op_return(&data.as_str().as_bytes())?, None)?;
                    let funds = tmpl.ctx().funds();
                    if funds.as_sat() != 0 {
                        tmpl = tmpl.add_output(funds, self, None)?;
                    }
                } else{
                    let funds = tmpl.ctx().funds();
                    tmpl = tmpl.add_output(funds, &self.pk, None)?;
                }
                tmpl.into()
        }
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
    /// # Update without Args
    NoUpdate {},
}
impl Default for UpdateTypes {
    fn default() -> Self {
        UpdateTypes::NoUpdate {}
    }
}
impl StatefulArgumentsTrait for UpdateTypes {}

impl Contract for ChainReturn {
    declare! {updatable<UpdateTypes>, Self::next_chain}
}
