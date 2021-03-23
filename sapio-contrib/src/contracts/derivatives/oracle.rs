// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A Price Oracle trait for Derivatives
use sapio_base::Clause;
/// Placeholder type for a standard way of looking up a stock symbol; can be defined more
/// concretely but should have a human readable string representation.
pub type Symbol = String;
/// Oracle is a generic wrapper for any logic to get a pair of binary clauses.
/// It can be based on hash preimage, federated signers, or key revealing.
/// The Trait Object can be responsible for network requests/caching.
pub trait Oracle {
    /// returns keys (price lo, price hi) for the given query
    fn get_key_lt_gte(&self, t: &Symbol, price: i64) -> (Clause, Clause);
}

/// An Oracle can also be "composed" into a threshold scheme with other
/// oracles quite easily as below...
///
/// Under *certain* circumstances, composition could be optimized (e.g., schnorr keys)
pub struct ThresholdOracle {
    /// the list of price oracles to consult
    pub oracles: Vec<Box<dyn Oracle>>,
    /// how many oracles must agree
    pub thresh: usize,
}

impl Oracle for ThresholdOracle {
    fn get_key_lt_gte(&self, t: &Symbol, price: i64) -> (Clause, Clause) {
        let (l, r) = self
            .oracles
            .iter()
            .map(|o| o.get_key_lt_gte(t, price))
            .unzip();
        (
            Clause::Threshold(self.thresh, l),
            Clause::Threshold(self.thresh, r),
        )
    }
}
