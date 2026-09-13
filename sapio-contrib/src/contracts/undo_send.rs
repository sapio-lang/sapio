// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Contracts useful for operations that should be revertible
use sapio_base::amount::CoinAmount;

use sapio::contract::*;
use sapio::*;
use sapio_base::timelocks::AnyRelTimeLock;

use schemars::*;
use serde::*;
use std::convert::TryInto;

/// # Undoable Sending Contract
/// UndoSendInternal allows funds to be sent to the to_contract only after a
/// relative timeout. Returning them to from_contract remains possible until
/// either spending transaction confirms.
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct UndoSendInternal {
    /// The contract to return funds to while the undo output is unspent
    pub from_contract: Compiled,
    /// the contract to forward funds to after timeout
    pub to_contract: Compiled,
    /// the amount
    // TODO: remove  and use ctx?
    pub amount: CoinAmount,
    /// the timeout period (relative height or blocks)
    pub timeout: AnyRelTimeLock,
}

impl UndoSendInternal {
    #[then]
    fn complete(self, ctx: sapio::Context) {
        ctx.template()
            .add_output(self.amount.try_into()?, &self.to_contract, None)?
            .set_sequence(0, self.timeout)?
            .into()
    }
    #[then]
    fn undo(self, ctx: sapio::Context) {
        ctx.template()
            .add_output(self.amount.try_into()?, &self.from_contract, None)?
            .into()
    }
}

impl Contract for UndoSendInternal {
    declare! {actions, Self::undo, Self::complete}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{context, key};
    use sapio_base::timelocks::RelHeight;

    #[test]
    fn undo_is_immediate_and_forwarding_waits_for_maturity() {
        let contract = UndoSendInternal {
            from_contract: key(1).compile(context(1000)).unwrap(),
            to_contract: key(2).compile(context(1000)).unwrap(),
            amount: bitcoin::Amount::from_sat(1000).into(),
            timeout: RelHeight::from(12).into(),
        };
        let object = contract.compile(context(1000)).unwrap();
        object.validate().unwrap();
        assert_eq!(object.ctv_to_tx.len(), 2);
        for template in object.ctv_to_tx.values() {
            let expected = if template.tx.input[0].sequence.to_consensus_u32() == 12 {
                &contract.to_contract
            } else {
                assert_eq!(template.tx.input[0].sequence.to_consensus_u32(), 1 << 22);
                &contract.from_contract
            };
            assert_eq!(template.tx.output[0].value.to_sat(), 1000);
            let actual: bitcoin::ScriptBuf = template.outputs[0].contract.address.clone().into();
            let expected: bitcoin::ScriptBuf = expected.address.clone().into();
            assert_eq!(actual, expected);
        }
        assert!(contract.compile(context(999)).is_err());
    }
}
