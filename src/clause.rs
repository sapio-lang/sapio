/// Concrete Instantiation of Miniscript Policy. Because we need to be able to generate exact
/// transactions, we only work with `bitcoin::PublicKey` types.
pub type Clause = miniscript::policy::concrete::Policy<bitcoin::PublicKey>;
