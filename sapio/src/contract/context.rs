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
use std::collections::HashMap;
use std::sync::Arc;
/// Context is used to track statet during compilation such as remaining value.
/// Context type is not copyable/clonable externally
#[derive(Clone)]
pub struct Context {
    /* TODO: Add Context Fields! */
    available_funds: Amount,
    emulator: Arc<dyn CTVEmulator>,
    /// which network is the contract building for?
    pub network: Network,
}

impl Context {
    /// create a context instance
    pub fn new(network: Network, amount: Amount, emulator: Arc<dyn CTVEmulator>) -> Self {
        Context {
            available_funds: amount,
            emulator: emulator,
            network,
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
    pub fn compile<A: Compilable>(&self, a: A) -> Result<Compiled, CompilationError> {
        a.compile(&self)
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
                ..*self
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
    pub fn template(&self) -> crate::template::Builder {
        crate::template::Builder::new(Context {
            emulator: self.emulator.clone(),
            ..*self
        })
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
