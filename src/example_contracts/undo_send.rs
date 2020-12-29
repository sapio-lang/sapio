use bitcoin::util::amount::CoinAmount;
use crate::clause::Clause;
use crate::contract::macros::*;
use crate::contract::*;
use crate::*;
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
            Ok(Box::new(iter::once(
                txn::TemplateBuilder::new()
                    .add_output(txn::Output::new(s.amount, s.to_contract.clone(), None)?)
                    .set_sequence(0, s.timeout)
                    .into(),
            )))
        }
    );
    then!(
        undo | s | {
            Ok(Box::new(iter::once(
                txn::TemplateBuilder::new()
                    .add_output(txn::Output::new(s.amount, s.from_contract.clone(), None)?)
                    .into(),
            )))
        }
    );
}

impl Contract for UndoSendInternal {
    declare! {then, Self::undo, Self::complete}
    declare! {non updatable}
}
