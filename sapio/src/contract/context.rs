// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! general non-parameter compilation state required by all contracts
use super::{Amount, Compilable, CompilationError, Compiled};
use crate::util::amountrange::AmountRange;
use crate::util::reverse_path::{MkReversePath, ReversePath};
use bitcoin::Network;
use miniscript::Descriptor;
use miniscript::DescriptorTrait;
use sapio_ctv_emulator_trait::CTVEmulator;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
/// Context is used to track statet during compilation such as remaining value.
pub struct Context {
    /* TODO: Add Context Fields! */
    available_funds: Amount,
    emulator: Arc<dyn CTVEmulator>,
    /// which network is the contract building for?
    pub network: Network,
    /// TODO: reversed linked list of ARCs to better de-duplicate memory.
    path: Arc<ReversePath<String>>,
}

lazy_static::lazy_static! {
    static ref CLONED : Arc<String> = Arc::new("cloned".into());
    static ref THEN_FN : Arc<String> = Arc::new("then_fn".into());
    static ref FINISH_OR_FN : Arc<String> = Arc::new("finish_or_fn".into());
    static ref FINISH_FN: Arc<String> = Arc::new("finish_fn".into());
    static ref CONDITIONAL_COMPILE_IF : Arc<String> = Arc::new("conditional_compile_if".into());
    static ref GUARD_FN : Arc<String> = Arc::new("guard_fn".into());
    static ref NEXT_TXS : Arc<String> = Arc::new("next_txs".into());
    static ref SUGGESTED_TXS : Arc<String> = Arc::new("suggested_txs".into());
    static ref INTERNED : HashMap<String, Arc<String>> = {
        let mut m = HashMap::<String, Arc<String>>::new();
        for s in [
            CLONED.clone(),
            THEN_FN.clone(),
            FINISH_OR_FN.clone(),
            FINISH_FN.clone(),
            CONDITIONAL_COMPILE_IF.clone(),
            GUARD_FN.clone(),
            NEXT_TXS.clone(),
            SUGGESTED_TXS.clone()]{
            m.insert(s.to_string(), s);
        }
        for i in 0..100 {
            m.insert(format!("{}", i),Arc::new(format!("{}", i)));
        }
        m
    };

}
fn get_interned(s: &str) -> Option<&Arc<String>> {
    (*INTERNED).get(&*s)
}

impl Context {
    /// create a context instance. Should only happen *once* at the very top
    /// level.
    pub fn new(
        network: Network,
        available_funds: Amount,
        emulator: Arc<dyn CTVEmulator>,
        path: Vec<Arc<String>>,
    ) -> Self {
        Context {
            available_funds,
            emulator,
            network,
            // TODO: Should return Option Self if path is not length > 0
            path: MkReversePath::from(path).unwrap(),
        }
    }

    /// Derive a new contextual path
    /// If no path is provided, it will be "cloned"
    pub fn derive<'a>(&self, path: Option<&'a str>) -> Self {
        let new_path = ReversePath::push(
            Option::Some(self.path.clone()),
            path.and_then(get_interned)
                .cloned()
                .or_else(|| path.map(String::from).map(Arc::new))
                .unwrap_or_else(|| CLONED.clone()),
        );
        Context {
            available_funds: self.available_funds,
            emulator: self.emulator.clone(),
            path: new_path,
            network: self.network,
        }
    }
    pub(crate) fn internal_clone(&self) -> Self {
        Context {
            available_funds: self.available_funds,
            emulator: self.emulator.clone(),
            path: self.path.clone(),
            network: self.network,
        }
    }

    /// return the available funds
    pub fn funds(&self) -> Amount {
        self.available_funds
    }

    /// use the context's emulator to get a emulated (or not) clause
    pub fn ctv_emulator(
        &self,
        b: bitcoin::hashes::sha256::Hash,
    ) -> Result<sapio_base::Clause, CompilationError> {
        Ok(self.emulator.get_signer_for(b)?)
    }

    /// Compile the compilable item with this context.
    pub fn compile<A: Compilable>(self, a: A) -> Result<Compiled, CompilationError> {
        a.compile(self)
    }

    // TODO: Fix
    /// return a context with the new amount if amount is smaller or equal to available
    pub fn with_amount(&self, amount: Amount) -> Result<Self, CompilationError> {
        if self.available_funds < amount {
            Err(CompilationError::OutOfFunds)
        } else {
            Ok(Context {
                available_funds: amount,
                emulator: self.emulator.clone(),
                path: self.path.clone(),
                network: self.network,
            })
        }
    }
    /// decrease the amount available in this context object.
    pub fn spend_amount(&mut self, amount: Amount) -> Result<(), CompilationError> {
        if self.available_funds < amount {
            Err(CompilationError::OutOfFunds)
        } else {
            self.available_funds -= amount;
            Ok(())
        }
    }

    /// Add funds to the context object (not typically needed)
    pub fn add_amount(&mut self, amount: Amount) {
        self.available_funds += amount;
    }

    /// Get a template builder from this context object
    pub fn template(self) -> crate::template::Builder {
        crate::template::Builder::new(self)
    }

    /// converts a descriptor and an optional AmountRange to a Object object.
    /// This can be used for e.g. creating raw SegWit Scripts.
    pub fn compiled_from_descriptor(
        d: Descriptor<bitcoin::PublicKey>,
        a: Option<AmountRange>,
    ) -> Compiled {
        Compiled {
            ctv_to_tx: HashMap::new(),
            suggested_txs: HashMap::new(),
            continue_apis: Default::default(),
            policy: None,
            address: d.address(bitcoin::Network::Bitcoin).unwrap().into(),
            descriptor: Some(d),
            amount_range: a.unwrap_or_else(|| {
                let mut a = AmountRange::new();
                a.update_range(Amount::min_value());
                a.update_range(Amount::from_sat(21_000_000 * 100_000_000));
                a
            }),
        }
    }
}
