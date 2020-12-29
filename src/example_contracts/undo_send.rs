use crate::clause::Clause;
use crate::contract::macros::*;
use crate::contract::*;
use crate::*;
use bitcoin::util::amount::CoinAmount;
use schemars::*;
use serde::*;
use std::iter;
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct UndoSendInternal {
    pub from_contract: Compiled,
    pub to_contract: Compiled,
    pub amount: CoinAmount,
    pub timeout: u32,
}

impl UndoSendInternal {
    then!(
        complete | s | {
            template::Builder::new()
                .add_output(template::Output::new(
                    s.amount,
                    s.to_contract.clone(),
                    None,
                )?)
                .set_sequence(0, s.timeout)
                .into()
        }
    );
    then!(
        undo | s | {
            template::Builder::new()
                .add_output(template::Output::new(
                    s.amount,
                    s.from_contract.clone(),
                    None,
                )?)
                .into()
        }
    );
}

impl Contract for UndoSendInternal {
    declare! {then, Self::undo, Self::complete}
    declare! {non updatable}
}
