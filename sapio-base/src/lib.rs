// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! base sapio library functionality and definitions, not particular to sapio compiler
#![deny(missing_docs)]
/// Extra functionality for working with Bitcoin types
pub mod util;
use bitcoin::XOnlyPublicKey;
pub use util::CTVHash;
pub mod plugin_args;
pub mod simp;

/// Helpers for making correct time locks
pub mod timelocks;
/// Trait & Structs for accessing Chain Data
pub mod txindex;

pub mod effects;
pub use effects::reverse_path;
pub mod serialization_helpers;

/// Concrete Instantiation of Miniscript Policy. Because we need to be able to generate exact
/// transactions, we only work with `bitcoin::PublicKey` types.
pub type Clause = miniscript::policy::concrete::Policy<XOnlyPublicKey>;
#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
