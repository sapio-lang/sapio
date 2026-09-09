// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! general non-parameter compilation state required by all contracts
use super::{Amount, Compilable, CompilationError, Compiled};
use crate::contract::compiler::InternalCompilerTag;
use crate::ordinals::Ordinal;
use crate::ordinals::OrdinalsInfo;

use bitcoin::Network;

use sapio_base::effects::EffectPath;
use sapio_base::effects::PathFragment;
pub use sapio_base::effects::{EffectDB, MapEffectDB};

use sapio_base::covenant::LoweringPlan;
use std::convert::TryInto;

use std::collections::HashSet;

use std::sync::Arc;

/// Context is used to track statet during compilation such as remaining value.
pub struct Context {
    /* TODO: Add Context Fields! */
    available_funds: Amount,
    lowering: LoweringPlan,
    /// which network is the contract building for?
    pub network: Network,
    /// TODO: reversed linked list of ARCs to better de-duplicate memory.
    path: Arc<EffectPath>,
    already_derived: HashSet<PathFragment>,
    effects: Arc<MapEffectDB>,
    ordinals_info: Option<OrdinalsInfo>,
}

fn allocate_ordinals(
    a: Amount,
    ords: &OrdinalsInfo,
) -> Result<[OrdinalsInfo; 2], CompilationError> {
    let mut amt = a.as_sat();
    let mut ret = [OrdinalsInfo(vec![]), OrdinalsInfo(vec![])];
    for (start, end) in ords.0.iter().copied() {
        let sats = end.0.checked_sub(start.0).ok_or_else(|| {
            CompilationError::OrdinalsError("ordinal range ends before its start".into())
        })?;
        let taken = amt.min(sats);
        let split = Ordinal(start.0 + taken);
        if taken > 0 {
            ret[0].0.push((start, split));
        }
        if taken < sats {
            ret[1].0.push((split, end));
        }
        amt -= taken;
    }
    if amt != 0 {
        return Err(CompilationError::OrdinalsError(
            "ordinal ranges do not cover the requested amount".into(),
        ));
    }
    Ok(ret)
}

impl Context {
    /// Borrow the Ordinals Info
    pub fn get_ordinals(&self) -> &Option<OrdinalsInfo> {
        &self.ordinals_info
    }
    /// create a context instance. Should only happen *once* at the very top
    /// level.
    pub fn new(
        network: Network,
        available_funds: Amount,
        lowering: LoweringPlan,
        path: EffectPath,
        effects: Arc<MapEffectDB>,
        ordinals_info: Option<OrdinalsInfo>,
    ) -> Self {
        Context {
            available_funds,
            lowering,
            network,
            // TODO: Should return Option Self if path is not length > 0
            path: Arc::new(path),
            already_derived: Default::default(),
            effects,
            ordinals_info,
        }
    }
    /// Get this Context's effect database, for clients
    pub unsafe fn get_effects_internal(&self) -> &Arc<MapEffectDB> {
        &self.effects
    }
    /// Get this Context's effect database
    pub(crate) fn get_effects(&self, _: InternalCompilerTag) -> &Arc<MapEffectDB> {
        &self.effects
    }
    /// Gets this Context's Path, but does not clone (left to caller)
    pub fn path(&self) -> &Arc<EffectPath> {
        &self.path
    }

    /// Derive a new contextual path
    pub fn derive_str<'a>(&mut self, path: Arc<String>) -> Result<Self, CompilationError> {
        let p: PathFragment = path.try_into()?;
        if matches!(p, PathFragment::Named(_)) {
            self.derive(p)
        } else {
            Err(CompilationError::InvalidPathName)
        }
    }
    /// Derive a new contextual path
    pub fn derive_num<T: Into<u64>>(&mut self, path: T) -> Result<Self, CompilationError> {
        self.derive(PathFragment::Branch(path.into()))
    }
    /// Derive a new contextual path
    pub(crate) fn derive(&mut self, path: PathFragment) -> Result<Self, CompilationError> {
        if self.already_derived.contains(&path) {
            Err(CompilationError::ContexPathAlreadyDerived)
        } else {
            self.already_derived.insert(path.clone());
            let new_path = EffectPath::push(Some(self.path.clone()), path);
            Ok(Context {
                available_funds: self.available_funds,
                lowering: self.lowering.clone(),
                path: new_path,
                network: self.network,
                already_derived: Default::default(),
                effects: self.effects.clone(),
                ordinals_info: self.ordinals_info.clone(),
            })
        }
    }
    /// return the available funds
    pub fn funds(&self) -> Amount {
        self.available_funds
    }

    /// Public, immutable policy-lowering inputs shared by nested compilation.
    /// Signer connections and host runtime state are not compiler inputs.
    pub fn lowering_plan(&self) -> &LoweringPlan {
        &self.lowering
    }

    /// Compile the compilable item with this context.
    pub fn compile<A: Compilable>(self, a: A) -> Result<Compiled, CompilationError> {
        a.compile(self)
    }

    /// return a context with the new amount if amount is smaller or equal to available
    pub fn with_amount(mut self, amount: Amount) -> Result<Self, CompilationError> {
        if self.available_funds < amount {
            Err(CompilationError::OutOfFunds)
        } else {
            self.available_funds = amount;
            self.ordinals_info = self
                .ordinals_info
                .as_ref()
                .map(|o| allocate_ordinals(amount, o).map(|[allocated, _]| allocated))
                .transpose()?;
            Ok(self)
        }
    }
    /// decrease the amount available in this context object.
    pub fn spend_amount(mut self, amount: Amount) -> Result<Self, CompilationError> {
        if self.available_funds < amount {
            Err(CompilationError::OutOfFunds)
        } else {
            self.available_funds -= amount;

            self.ordinals_info = self
                .ordinals_info
                .as_ref()
                .map(|o| allocate_ordinals(amount, o).map(|[_, remaining]| remaining))
                .transpose()?;
            Ok(self)
        }
    }

    /// Add external funds whose ordinal ranges are unknown.
    ///
    /// Allocate all tracked input sats first. Clearing exhausted tracking at
    /// this boundary prevents unknown inputs from acquiring invented ordinal
    /// identities or shifting a known ordinal into the wrong output.
    pub fn add_amount(mut self, amount: Amount) -> Result<Self, CompilationError> {
        let available = self
            .available_funds
            .as_sat()
            .checked_add(amount.as_sat())
            .ok_or_else(|| CompilationError::TerminateWith("Available funds overflow".into()))?;
        if amount != Amount::ZERO {
            if let Some(ordinals) = &self.ordinals_info {
                if self.available_funds != Amount::ZERO || !ordinals.0.is_empty() {
                    return Err(CompilationError::OrdinalsError(
                        "Allocate all tracked sats before adding unknown external inputs".into(),
                    ));
                }
                self.ordinals_info = None;
            }
        }
        self.available_funds = Amount::from_sat(available);
        Ok(self)
    }

    /// Get a template builder from this context object
    pub fn template(self) -> crate::template::Builder {
        crate::template::Builder::new(self)
    }
}
