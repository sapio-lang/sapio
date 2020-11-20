use super::undo_send::UndoSendInternal;
use bitcoin::util::amount::CoinAmount;
use sapio::clause::Clause;
use sapio::contract::macros::*;
use sapio::contract::*;
use sapio::contract::{DynamicContract, DynamicContractRef};
use sapio::*;
use schemars::*;
use serde::*;
use std::convert::TryFrom;
use std::marker::PhantomData;
use std::rc::Rc;

#[derive(JsonSchema, Deserialize)]
pub struct DynamicExample {
    cold_storage: bitcoin::Address,
    max_per_address: CoinAmount,
    radix: usize,
    hot_storage: bitcoin::Address,
    n_steps: u64,
    amount_step: CoinAmount,
    timeout: u32,
    mature: u32,
}

struct D<'a> {
    v: Vec<fn() -> Option<actions::ThenFunc<'a, D<'a>>>>
}

impl<'a> AnyContract<'a> for D<'a> {
    type StatefulArguments = ();
    type Ref = Self;
    fn then_fns(&'a self) -> &'a [fn() -> Option<actions::ThenFunc<'a, Self>>] {
        &self.v
    }
    fn finish_or_fns(
        &'a self,
    ) -> &'a [fn() -> Option<actions::FinishOrFunc<'a, Self, Self::StatefulArguments>>] {
        &[]
    }
    fn finish_fns(&'a self) -> &'a [fn() -> Option<actions::Guard<Self>>] {
        &[]
    }
    fn get_inner_ref(&self) -> &Self {
        self
    }
}
impl<'a> DynamicExample {
    then! {next |s| {
        let v:
            Vec<fn() -> Option<actions::ThenFunc<'a, D<'a>>>>
            = vec![];
        let d : D<'a> = D{v};
        let mut builder = txn::TemplateBuilder::new()
        .add_output(txn::Output::new(s.amount_step.into(),
        <D<'a> as Compilable>::compile(&d), None)?);

        Ok(Box::new(std::iter::once(builder.into())))
    }}
}

impl<'a> Contract<'a> for DynamicExample {
    declare! {then, Self::next}
    declare! {non updatable}
}
