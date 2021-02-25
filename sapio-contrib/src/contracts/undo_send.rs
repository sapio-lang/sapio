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
        complete | s,
        ctx | {
            ctx.template()
                .add_output(s.amount.try_into()?, &s.to_contract, None)?
                .set_sequence(0, s.timeout)?
                .into()
        }
    );
    then!(
        undo | s,
        ctx | {
            ctx.template()
                .add_output(s.amount.try_into()?, &s.from_contract, None)?
                .into()
        }
    );
}

impl Contract for UndoSendInternal {
    declare! {then, Self::undo, Self::complete}
    declare! {non updatable}
}
