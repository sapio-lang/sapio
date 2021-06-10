// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! coin_pool has a contract `CoinPool` for sharing a UTXO
use bitcoin::Amount;
use sapio_base::Clause;

use sapio::contract::*;
use sapio::*;

use sapio_base::timelocks::AnyRelTimeLock;
use std::sync::{Arc, Mutex};
type Payouts = Vec<(Arc<Mutex<dyn Compilable>>, Amount)>;

/// A CoinPool is a contract that allows a group of individuals to
/// cooperatively share a UTXO.
pub struct CoinPool {
    clauses: Vec<Clause>,
    refunds: Payouts,
}

impl CoinPool {
    then! {
        /// cuts the pool in half in order to remove an offline or malicious participant
        fn bisect_offline(self, ctx) {
            if self.clauses.len() >= 2 {
                let l = self.clauses.len();
                let a = CoinPool {
                    clauses: self.clauses[0..l/2].into(),
                    refunds: self.refunds[0..l/2].into()
                };

                let b = CoinPool {
                    clauses: self.clauses[l/2..].into(),
                    refunds: self.refunds[l/2..].into()
                };

                ctx.template().add_output(
                    Amount::from_sat(a.refunds.iter().map(|x| x.1.as_sat()).sum()),
                    &a,
                    None
                )?.add_output(
                    Amount::from_sat(b.refunds.iter().map(|x| x.1.as_sat()).sum()),
                    &b,
                    None
                )?.into()
            } else {
                let mut builder = ctx.template();
                for (cmp, amt) in self.refunds.iter() {
                builder = builder.add_output(*amt, &*cmp.lock().unwrap(), None)?;
                }
                builder.into()
            }
        }
    }
    guard! {
        /// everyone has signed off on the transaction
        fn all_approve(self, _ctx) {
            Clause::Threshold(self.clauses.len(), self.clauses.clone())
        }
    }
    finish! {
        /// move the coins to the next state -- payouts may recursively contain pools itself
        guarded_by: [Self::all_approve]
        fn next_pool(self, ctx, o) {
            if let Some(coin_pool) = o {
                let mut tmpl = ctx.template().add_amount(coin_pool.external_amount);
                for (to, amt) in coin_pool.payouts.iter() {
                    tmpl = tmpl.add_output(*amt, &*to.lock().unwrap(), None)?;
                }
                for seq in coin_pool.add_inputs.iter() {
                    tmpl = tmpl.add_sequence().set_sequence(-1, *seq)?;
                }
                tmpl.into()
            } else {
                Ok(Box::new(std::iter::empty()))
            }
        }
    }
}

/// `CoinPoolUpdate` allows updating a `CoinPool` to a new state.
pub struct CoinPoolUpdate {
    /// the contracts to pay into
    payouts: Payouts,
    /// if we should add any inputs to the transaction, and if so, what the
    /// sequences should be set to.
    add_inputs: Vec<AnyRelTimeLock>,
    /// If the external inputs are contributing funds -- this allows two
    /// coinpools to merge.
    /// TODO: Allow different indexes?
    external_amount: Amount,
}

impl Contract for CoinPool {
    declare! {then, Self::bisect_offline}
    declare! {finish, Self::all_approve}
    declare! {updatable<CoinPoolUpdate>, Self::next_pool}
}
