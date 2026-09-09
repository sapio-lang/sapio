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

/// Demonstrates how to make a contract object without known functionality at
/// (rust) compile time. `D` Binds statically to the AnyContract interface though!
struct D<'a> {
    v: Vec<fn() -> Option<actions::ThenFuncAsFinishOrFunc<'a, D<'a>, ()>>>,
    key: bitcoin::XOnlyPublicKey,
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
        &[|| {
            Some(actions::Guard::Fresh(
                |s, _| sapio_base::Clause::Key(s.key),
                None,
            ))
        }]
    }
    fn get_inner_ref<'a>(&'a self) -> &'a Self {
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
pub struct DynamicExample {
    /// Key controlling both dynamically constructed outputs.
    #[schemars(with = "String")]
    key: bitcoin::XOnlyPublicKey,
}
impl DynamicExample {
    #[then]
    fn next(self, ctx: sapio::Context) {
        let v: Vec<fn() -> Option<actions::ThenFuncAsFinishOrFunc<'static, D<'static>, ()>>> =
            vec![];
        let d: D<'_> = D { v, key: self.key };

        let d2 = DynamicContract::<(), bitcoin::XOnlyPublicKey> {
            then: vec![|| None],
            finish: vec![|| {
                Some(actions::Guard::Fresh(
                    |key, _| sapio_base::Clause::Key(*key),
                    None,
                ))
            }],
            finish_or: vec![],
            data: self.key,
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

    fn ensure_amount(&self, ctx: Context) -> Result<Amount, CompilationError> {
        if ctx.funds().as_sat() < 2 {
            return Err(CompilationError::OutOfFunds);
        }
        Ok(ctx.funds())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{context, key};

    #[test]
    fn dynamic_outputs_are_spendable_and_preserve_odd_balances() {
        let object = DynamicExample { key: key(1) }
            .compile(context(1001))
            .unwrap();
        object.validate().unwrap();
        let outputs = &object.ctv_to_tx.values().next().unwrap().outputs;
        assert_eq!(
            outputs
                .iter()
                .map(|o| o.amount.as_sat())
                .collect::<Vec<_>>(),
            vec![500, 501]
        );
        assert!(outputs.iter().all(|o| o.contract.descriptor.is_some()));
        assert!(DynamicExample { key: key(1) }.compile(context(1)).is_err());
    }
}
