// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! example of using a dynamic contract
use bitcoin::Amount;
use sapio::contract::object::ObjectMetadata;
use sapio::contract::DynamicContract;
use sapio::contract::*;
use sapio::*;
use schemars::*;
use serde::*;
use std::sync::Arc;

/// Demonstrates how to make a contract object without known functionality at
/// (rust) compile time. `D` Binds statically to the AnyContract interface though!
struct D<'a> {
    v: Vec<fn() -> Option<actions::ThenFuncAsFinishOrFunc<'a, D<'a>, ()>>>,
}

impl AnyContract for D<'static> {
    type StatefulArguments = ();
    type Ref = Self;
    fn then_fns<'a>(
        &'a self,
    ) -> &'a [fn() -> Option<actions::ThenFuncAsFinishOrFunc<'a, Self, Self::StatefulArguments>>]
    where
        Self::Ref: 'a,
    {
        &self.v
    }
    fn finish_or_fns<'a>(
        &'a self,
    ) -> &'a [fn() -> Option<Box<dyn actions::CallableAsFoF<Self, Self::StatefulArguments>>>] {
        &[]
    }
    fn finish_fns<'a>(&'a self) -> &'a [fn() -> Option<actions::Guard<Self>>] {
        &[]
    }
    fn get_inner_ref<'a>(&'a self) -> &Self {
        self
    }
    fn metadata<'a>(&'a self, _ctx: Context) -> Result<ObjectMetadata, CompilationError> {
        Ok(Default::default())
    }
    fn ensure_amount<'a>(&'a self, _ctx: Context) -> Result<Amount, CompilationError> {
        Ok(Amount::from_sat(0))
    }
}

/// Shows how to make a Dynamic Contract without creating a bespoke type.
#[derive(JsonSchema, Deserialize)]
pub struct DynamicExample;
impl DynamicExample {
    #[then]
    fn next(self, ctx: sapio::Context) {
        let v: Vec<fn() -> Option<actions::ThenFuncAsFinishOrFunc<'static, D<'static>, ()>>> =
            vec![];
        let d: D<'_> = D { v };

        let d2 = DynamicContract::<(), String> {
            then: vec![|| None, || {
                Some(
                    sapio::contract::actions::ThenFunc {
                        conditional_compile_if: &[],
                        guard: &[],
                        func: |_s, _ctx, _t| Err(CompilationError::TerminateCompilation),
                        name: Arc::new("Empty".into()),
                    }
                    .into(),
                )
            }],
            finish: vec![],
            finish_or: vec![],
            data: "E.g., Create a Vault".into(),
            metadata_f: Box::new(|_s, _c| Ok(Default::default())),
            ensure_amount_f: Box::new(|_s, _c| Ok(Default::default())),
        };
        let mut bld = ctx.template();
        let amt = bld.ctx().funds() / 2;
        bld = bld.add_output(amt, &d, None)?;
        let amt2 = bld.ctx().funds();
        bld.add_output(amt2, &d2, None)?.into()
    }
}

impl Contract for DynamicExample {
    declare! {then, Self::next}
    declare! {non updatable}
}
