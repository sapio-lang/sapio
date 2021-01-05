use bitcoin::util::amount::CoinAmount;

use sapio::contract::*;
use sapio::*;
use schemars::*;
use serde::*;
use std::convert::TryInto;

use sapio_base::timelocks::AnyRelTimeLock;

#[derive(JsonSchema, Serialize, Deserialize)]
pub struct UndoSendInternal {
    pub from_contract: Compiled,
    pub to_contract: Compiled,
    pub amount: CoinAmount,
    pub timeout: AnyRelTimeLock,
}

impl UndoSendInternal {
    then!(
        complete | s,
        ctx | {
            ctx.template()
                .add_output(s.amount.try_into()?, &s.to_contract, None)?
                .set_sequence(0, s.timeout)
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
