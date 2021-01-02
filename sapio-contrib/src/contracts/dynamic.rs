use super::undo_send::UndoSendInternal;
use sapio::clause::Clause;
use sapio::contract::macros::*;
use sapio::contract::DynamicContract;
use sapio::contract::*;
use sapio::*;
use bitcoin::util::amount::CoinAmount;
use schemars::*;
use serde::*;
use std::convert::{TryFrom, TryInto};
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

struct D {
    v: Vec<fn() -> Option<actions::ThenFunc<D>>>,
}

impl AnyContract for D {
    type StatefulArguments = ();
    type Ref = Self;
    fn then_fns<'a>(&'a self) -> &'a [fn() -> Option<actions::ThenFunc<Self>>] {
        &self.v
    }
    fn finish_or_fns<'a>(
        &'a self,
    ) -> &'a [fn() -> Option<actions::FinishOrFunc<Self, Self::StatefulArguments>>] {
        &[]
    }
    fn finish_fns<'a>(&'a self) -> &'a [fn() -> Option<actions::Guard<Self>>] {
        &[]
    }
    fn get_inner_ref<'a>(&'a self) -> &Self {
        self
    }
}
impl DynamicExample {
    then! {next |s, ctx| {
        let v:
            Vec<fn() -> Option<actions::ThenFunc<D>>>
            = vec![];
        let d : D = D{v};

        let d2 = DynamicContract::<(), String> {
            then: vec![|| None, || Some(sapio::contract::actions::ThenFunc{guard: &[], func: |s, ctx| Err(CompilationError::TerminateCompilation)})],
            finish: vec![],
            finish_or: vec![],
            data: "E.g., Create a Vault".into(),
        };
        ctx.template()
        .add_output(s.amount_step.try_into()?, &d, None)?
        .add_output(s.amount_step.try_into()?, &d2, None)?
        .into()
    }}
}

impl Contract for DynamicExample {
    declare! {then, Self::next}
    declare! {non updatable}
}
