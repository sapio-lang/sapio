use bitcoin::util::amount::CoinAmount;
use sapio::clause::Clause;
use sapio::contract::macros::*;
use sapio::contract::*;
use sapio::*;
use schemars::*;
use serde::*;
use super::undo_send::UndoSendInternal;
use std::rc::Rc;
use std::marker::PhantomData;
use std::convert::TryFrom;


#[derive(JsonSchema, Deserialize)]
pub struct Vault {
    cold_storage: bitcoin::Address,
    hot_storage: bitcoin::Address,
    n_steps: u64,
    amount_step: CoinAmount,
    timeout: u32,
    mature: u32,
}

impl<'a> Vault {
    then! {step |s| {
        let mut builder = txn::TemplateBuilder::new()
        .add_output(txn::Output::new(s.amount_step.into(),
                UndoSendInternal {
                    from_contract: Compiled::from_address(s.cold_storage.clone(), None),
                    to_contract: Compiled::from_address(s.hot_storage.clone(), None),
                    timeout: s.mature,
                    amount: s.amount_step.into(),
                }, None)?);

        Ok(Box::new(std::iter::once(
        if s.n_steps > 1 {
            let sub_amount = bitcoin::Amount::try_from(s.amount_step).map_err(|e| contract::CompilationError::TerminateCompilation)?.checked_mul(s.n_steps - 1).ok_or(contract::CompilationError::TerminateCompilation)?;
            let sub_vault = Vault {
                cold_storage: s.cold_storage.clone(),
                hot_storage: s.hot_storage.clone(),
                n_steps: s.n_steps -1,
                amount_step: s.amount_step,
                timeout: s.timeout,
                mature: s.mature,

            }.compile()?;
            builder.add_output(txn::Output::new(sub_amount.into(), sub_vault, None)?)
        } else {
            builder
        }.into()
        )))

    }}
    then!{to_cold |s| {
        let amount = bitcoin::Amount::try_from(s.amount_step).map_err(|e| contract::CompilationError::TerminateCompilation)?.checked_mul(s.n_steps).ok_or(contract::CompilationError::TerminateCompilation)?;
        let mut builder = txn::TemplateBuilder::new()
            .add_output(txn::Output::new(amount.into(), Compiled::from_address(s.cold_storage.clone(), None), None)?);
        Ok(Box::new(std::iter::once(builder.into())))

    }}
}

impl<'a> Contract<'a> for Vault {
    declare! {then, Self::step, Self::to_cold}
    declare! {non updatable}
}

