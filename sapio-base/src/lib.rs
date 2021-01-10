/// Extra functionality for working with Bitcoin types
pub mod util;
pub use util::CTVHash;

/// Helpers for making correct time locks
pub mod timelocks;
/// Trait & Structs for accessing Chain Data
pub mod txindex;

/// Concrete Instantiation of Miniscript Policy. Because we need to be able to generate exact
/// transactions, we only work with `bitcoin::PublicKey` types.
pub type Clause = miniscript::policy::concrete::Policy<bitcoin::PublicKey>;
#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
