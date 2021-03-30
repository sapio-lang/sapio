// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Contracts useful for operations that should be revertible
use bitcoin::util::amount::CoinAmount;

use sapio::contract::*;
use sapio::*;
use schemars::*;
use serde::*;
use std::convert::TryInto;

use sapio_base::timelocks::AnyRelTimeLock;

/// UndoSendInternal allows funds to be sent to the to_contract only after a
/// relative timeout. Otherwise, they can move back to the from_contract.
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct UndoSendInternal {
    /// The contract to return funds to before timeout
    pub from_contract: Compiled,
    /// the contract to forward funds to after timeout
    pub to_contract: Compiled,
    /// the amount
    /// TODO: remove  and use ctx?
    pub amount: CoinAmount,
    /// the timeout period (relative height or blocks)
    pub timeout: AnyRelTimeLock,
}

impl UndoSendInternal {
    then!(
        fn complete(self, ctx) {
            ctx.template()
                .add_output(self.amount.try_into()?, &self.to_contract, None)?
                .set_sequence(0, self.timeout)?
                .into()
        }
    );
    then!(
        fn undo (self, ctx) {
            ctx.template()
                .add_output(self.amount.try_into()?, &self.from_contract, None)?
                .into()
        }
    );
}

impl Contract for UndoSendInternal {
    declare! {then, Self::undo, Self::complete}
    declare! {non updatable}
}
