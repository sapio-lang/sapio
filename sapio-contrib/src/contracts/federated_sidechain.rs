// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A Contract that offers peg-in functionality for sidechains
use sapio::contract::*;
use sapio::*;
use sapio_base::amount::CoinAmount;
use sapio_base::miniscript::{Threshold, ThresholdError};
use sapio_base::Clause;
use sapio_macros::guard;
use schemars::*;
use serde::*;
use std::convert::TryInto;
use std::marker::PhantomData;

/// State  when recover may start
#[derive(JsonSchema, Deserialize, Default)]
pub struct CanBeginRecovery;
/// State when recovery may complete
#[derive(JsonSchema, Deserialize, Default)]
pub struct CanFinishRecovery;

/// trait-level enum for states of a FederatedPegIn
pub trait RecoveryState {}

impl RecoveryState for CanFinishRecovery {}
impl RecoveryState for CanBeginRecovery {}

#[derive(JsonSchema, Deserialize)]
/// A contract for depositing into a federated side chain.
pub struct FederatedPegIn<T: RecoveryState> {
    /// # Normal Operation Keys
    keys: Vec<bitcoin::XOnlyPublicKey>,
    /// # Normal Operation Threshold
    thresh_normal: usize,
    /// # Recovery Operation Keys
    keys_recovery: Vec<bitcoin::XOnlyPublicKey>,
    /// # Recovery Operation Threshold
    thresh_recovery: usize,
    /// # Amount to Deposit
    amount: CoinAmount,
    #[serde(skip, default)]
    _pd: PhantomData<T>,
}

/// Actions that will be specialized depending on the exact state.
pub trait StateDependentActions
where
    Self: Sized + Contract,
{
    decl_guard! {
    /// Should only be defined when RecoveryState is in CanFinishRecovery
    policy finish_recovery<Result<Clause, ThresholdError>>}

    decl_then! {
    /// Should only be defined when RecoveryState is in CanBeginRecovery
    begin_recovery}
}
impl StateDependentActions for FederatedPegIn<CanBeginRecovery> {
    #[then(guarded_by = "[Self::recovery_signed]")]
    fn begin_recovery(self, ctx: sapio::Context) {
        ctx.template()
            .add_output(
                self.amount.try_into()?,
                &FederatedPegIn::<CanFinishRecovery> {
                    keys: self.keys.clone(),
                    thresh_normal: self.thresh_normal,
                    keys_recovery: self.keys_recovery.clone(),
                    thresh_recovery: self.thresh_recovery,
                    amount: self.amount,
                    _pd: PhantomData::default(),
                },
                None,
            )?
            .into()
    }
}
impl StateDependentActions for FederatedPegIn<CanFinishRecovery> {
    #[guard(policy)]
    fn finish_recovery(self, _ctx: Context) -> Result<Clause, ThresholdError> {
        Ok(Clause::And(vec![
            Clause::try_from(sapio_base::timelocks::RelHeight::from(4725))
                .expect("positive constant locktime")
                .into(),
            Clause::Thresh(Threshold::new(
                self.thresh_recovery,
                self.keys_recovery
                    .iter()
                    .cloned()
                    .map(Clause::Key)
                    .map(Into::into)
                    .collect(),
            )?)
            .into(),
        ]))
    }
}

impl<T: RecoveryState> FederatedPegIn<T> {
    #[guard(policy)]
    fn recovery_signed(self, _ctx: Context) -> Result<Clause, ThresholdError> {
        Ok(Clause::Thresh(Threshold::new(
            self.thresh_recovery,
            self.keys_recovery
                .iter()
                .cloned()
                .map(Clause::Key)
                .map(Into::into)
                .collect(),
        )?))
    }

    #[guard(policy)]
    fn normal_signed(self, _ctx: Context) -> Result<Clause, ThresholdError> {
        Ok(Clause::Thresh(Threshold::new(
            self.thresh_normal,
            self.keys
                .iter()
                .cloned()
                .map(Clause::Key)
                .map(Into::into)
                .collect(),
        )?))
    }
}

impl<T: RecoveryState> Contract for FederatedPegIn<T>
where
    FederatedPegIn<T>: StateDependentActions + 'static,
{
    declare! {then, Self::begin_recovery}
    declare! {finish, Self::normal_signed, Self::finish_recovery}
    declare! {non updatable}

    fn ensure_amount(&self, _ctx: Context) -> Result<bitcoin::Amount, CompilationError> {
        if self.thresh_normal == 0
            || self.thresh_normal > self.keys.len()
            || self.thresh_recovery == 0
            || self.thresh_recovery > self.keys_recovery.len()
        {
            return Err(CompilationError::Custom(
                "Federation thresholds must select available keys".into(),
            ));
        }
        Ok(self.amount.try_into()?)
    }
}

/// Type Alias for the state to start FederatedPegIn from.
pub type PegIn = FederatedPegIn<CanBeginRecovery>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{context, key};

    fn peg() -> PegIn {
        FederatedPegIn {
            keys: vec![key(1), key(2)],
            thresh_normal: 2,
            keys_recovery: vec![key(3), key(4)],
            thresh_recovery: 1,
            amount: bitcoin::Amount::from_sat(1000).into(),
            _pd: PhantomData,
        }
    }

    #[test]
    fn recovery_preserves_funds_and_requires_recovery_keys_after_delay() {
        let contract = peg();
        let object = contract.compile(context(1000)).unwrap();
        object.validate().unwrap();
        let recovery = object.ctv_to_tx.values().next().unwrap();
        assert_eq!(recovery.tx.output[0].value.to_sat(), 1000);
        assert_eq!(
            contract.guard_recovery_signed(context(0)).unwrap(),
            Clause::Thresh(
                Threshold::new(
                    1,
                    vec![Clause::Key(key(3)).into(), Clause::Key(key(4)).into()]
                )
                .expect("valid threshold")
            )
        );
        let finish = FederatedPegIn::<CanFinishRecovery> {
            keys: contract.keys.clone(),
            thresh_normal: 2,
            keys_recovery: contract.keys_recovery.clone(),
            thresh_recovery: 1,
            amount: contract.amount,
            _pd: PhantomData,
        };
        assert_eq!(
            finish.guard_finish_recovery(context(0)).unwrap(),
            Clause::And(vec![
                Clause::try_from(sapio_base::timelocks::RelHeight::from(4725))
                    .expect("positive constant locktime")
                    .into(),
                contract.guard_recovery_signed(context(0)).unwrap().into()
            ])
        );
        assert!(recovery.outputs[0].contract.ctv_to_tx.is_empty());
    }

    #[test]
    fn invalid_thresholds_and_missing_funds_fail() {
        let mut contract = peg();
        contract.thresh_normal = 0;
        assert!(contract.guard_normal_signed(context(1000)).is_err());
        assert!(contract.compile(context(1000)).is_err());
        contract.thresh_normal = 2;
        contract.thresh_recovery = 3;
        assert!(contract.guard_recovery_signed(context(1000)).is_err());
        assert!(contract.compile(context(1000)).is_err());
        assert!(peg().compile(context(999)).is_err());
    }

    #[test]
    fn wasm_catalog_fixture_allows_a_key_in_both_federations() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contrib/vectors/examples/fedpeg.json"
        ))
        .unwrap();
        let contract: PegIn = serde_json::from_value(fixture["arguments"].clone()).unwrap();
        assert_eq!(contract.keys_recovery.len(), 1);
        assert!(contract.keys.contains(&contract.keys_recovery[0]));
        let object = contract
            .compile(context(fixture["context"]["amount"].as_u64().unwrap()))
            .unwrap();
        object.validate().unwrap();
        assert_eq!(object.ctv_to_tx.len(), 1);
        let recovery = object.ctv_to_tx.values().next().unwrap();
        assert_eq!(recovery.tx.output[0].value.to_sat(), 1000);
        let recovered = &recovery.outputs[0].contract;
        assert!(recovered.ctv_to_tx.is_empty());
        recovered.validate().unwrap();
    }
}
