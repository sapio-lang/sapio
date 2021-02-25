//! External required APIs in order to use Derivative contracts
use super::*;
/// An API for the Operator Service:
pub trait OperatorApi {
    /// Return Operator's Oracle
    fn get_oracle(&self) -> &dyn Oracle;
    /// Get a fresh key clause for Operator signing (could be a multisig etc)
    fn get_key(&self) -> Clause;
    /// Get a contract for a receivable amount. Allows Operator to direct funds to e.g.
    /// cold storage contracts
    fn receive_payment(&self, amount: Amount) -> Compiled;
}

/// An API for the Counterparty
pub trait UserApi {
    /// Get a fresh key clause for user signing (could be a multisig etc)
    fn get_key(&self) -> Clause;
    /// Get a contract for a receivable amount. Allows Userto direct funds to e.g.
    /// cold storage contracts
    fn receive_payment(&self, amount: Amount) -> Compiled;
}
