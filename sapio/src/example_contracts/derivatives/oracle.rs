use crate::clause::Clause;
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
    pub oracles: Vec<Box<dyn Oracle>>,
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
