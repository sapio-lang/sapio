// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! general non-parameter compilation state required by all contracts
use super::{Amount, Compilable, CompilationError, Compiled};
use crate::util::amountrange::AmountRange;
use bitcoin::Network;
use miniscript::Descriptor;
use miniscript::DescriptorTrait;
use sapio_ctv_emulator_trait::CTVEmulator;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
/// Used to Build a Shared Path for all children of a given context.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
#[serde(try_from = "Vec<String>")]
#[serde(into = "Vec<String>")]
pub struct ReversePath {
    past: Option<Arc<ReversePath>>,
    this: Arc<String>,
}

use std::convert::TryFrom;
impl TryFrom<Vec<String>> for ReversePath {
    type Error = &'static str;
    fn try_from(v: Vec<String>) -> Result<ReversePath, Self::Error> {
        let mut rp = None;
        for val in v {
            rp = Some(ReversePath::push(rp, Arc::new(val)));
        }
        if let Option::Some(v) = rp {
            // Arc unwrap never fail!
            Ok(Arc::try_unwrap(v).unwrap())
        } else {
            Err("Reverse Path must have at least one element.")
        }
    }
}
impl From<ReversePath> for Vec<String> {
    fn from(r: ReversePath) -> Self {
        let mut result: Vec<String> = vec![(*r.this).clone()];
        let mut node = &r.past;
        while let Some(v) = node {
            result.push((*v.this).clone());
            node = &v.past;
        }
        result.reverse();
        result
    }
}
impl From<ReversePath> for Vec<Arc<String>> {
    fn from(r: ReversePath) -> Self {
        let mut result = vec![r.this];
        let mut node = &r.past;
        while let Some(v) = node {
            result.push(v.this.clone());
            node = &v.past;
        }
        result.reverse();
        result
    }
}
struct MK(Option<Arc<ReversePath>>);
impl MK {
    fn unwrap(self) -> Arc<ReversePath> {
        if let Some(x) = self.0 {
            x
        } else {
            panic!("Vector must have at least one root path")
        }
    }
}
impl From<Vec<Arc<String>>> for MK {
    fn from(v: Vec<Arc<String>>) -> Self {
        let mut rp: Option<Arc<ReversePath>> = None;
        for val in v {
            let new: Arc<ReversePath> = ReversePath::push(rp, val);
            rp = Some(new);
        }
        MK(rp)
    }
}
impl ReversePath {
    fn push(v: Option<Arc<ReversePath>>, s: Arc<String>) -> Arc<ReversePath> {
        Arc::new(ReversePath { past: v, this: s })
    }
}
/// Context is used to track statet during compilation such as remaining value.
pub struct Context {
    /* TODO: Add Context Fields! */
    available_funds: Amount,
    emulator: Arc<dyn CTVEmulator>,
    /// which network is the contract building for?
    pub network: Network,
    /// TODO: reversed linked list of ARCs to better de-duplicate memory.
    path: Arc<ReversePath>,
}

lazy_static::lazy_static! {
    static ref CLONED : Arc<String> = Arc::new("cloned".into());
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
            path: MK::from(path).unwrap(),
        }
    }

    /// Derive a new contextual path
    /// If no path is provided, it will be "cloned"
    pub fn derive<'a>(&self, path: Option<&'a str>) -> Self {
        let new_path = ReversePath::push(
            Option::Some(self.path.clone()),
            path.map(String::from)
                .map(Arc::new)
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
