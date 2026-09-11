// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! base sapio library functionality and definitions, not particular to sapio compiler
#![deny(missing_docs)]
pub mod amount;
/// Extra functionality for working with Bitcoin types
pub mod util;
use bitcoin::XOnlyPublicKey;
pub use miniscript;
pub use util::CTVHash;
pub mod covenant;
mod crypto;
pub mod fragments;
pub mod plugin_args;
pub mod policy;
pub mod program;
pub mod simp;
pub use covenant::{CovenantError, Ctv, Emulatable, LoweringPlan};
pub use program::{EmulatedProgram, EvaluatorId, ProgramId, ProgramInstance};

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
