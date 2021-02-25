//! example of using a dynamic contract
use sapio::contract::DynamicContract;
use sapio::contract::*;
use sapio::*;
use schemars::*;
use serde::*;

/// Demonstrates how to make a contract object without known functionality at
/// (rust) compile time. `D` Binds statically to the AnyContract interface though!
struct D<'a> {
    v: Vec<fn() -> Option<actions::ThenFunc<'a, D<'a>>>>,
}

impl AnyContract for D<'static> {
    type StatefulArguments = ();
    type Ref = Self;
    fn then_fns<'a>(&'a self) -> &'a [fn() -> Option<actions::ThenFunc<'a, Self>>]
    where
        Self::Ref: 'a,
    {
        &self.v
    }
    fn finish_or_fns<'a>(
        &'a self,
    ) -> &'a [fn() -> Option<actions::FinishOrFunc<'a, Self, Self::StatefulArguments>>] {
        &[]
    }
    fn finish_fns<'a>(&'a self) -> &'a [fn() -> Option<actions::Guard<Self>>] {
        &[]
    }
    fn get_inner_ref<'a>(&'a self) -> &Self {
        self
    }
}

/// Shows how to make a Dynamic Contract without creating a bespoke type.
#[derive(JsonSchema, Deserialize)]
pub struct DynamicExample;
impl DynamicExample {
    then! {next |s, ctx| {
        let v:
            Vec<fn() -> Option<actions::ThenFunc<'static, D<'static>>>>
            = vec![];
        let d : D = D{v};

        let d2 = DynamicContract::<(), String> {
            then: vec![|| None, || Some(sapio::contract::actions::ThenFunc{guard: &[], func: |_s, _ctx| Err(CompilationError::TerminateCompilation)})],
            finish: vec![],
            finish_or: vec![],
            data: "E.g., Create a Vault".into(),
        };
        ctx.template()
        .add_output(ctx.funds()/2, &d, None)?
        .add_output(ctx.funds()/2, &d2, None)?
        .into()
    }}
}

impl Contract for DynamicExample {
    declare! {then, Self::next}
    declare! {non updatable}
}
