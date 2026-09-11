// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Validate source policies before conjunction simplification or leaf splitting.

use crate::contract::CompilationError;
use sapio_base::miniscript::policy::{compiler::CompilerError, concrete::PolicyError};
use sapio_base::Clause;

/// Check every source node, including nodes that optimization would discard.
/// Duplicate-key and combined timelock checks belong to each eventual script:
/// separate alternatives may legitimately repeat a key.
pub(crate) fn validate_policy(policy: &Clause) -> Result<(), CompilationError> {
    match policy {
        Clause::And(children) => {
            if children.len() != 2 {
                return Err(CompilerError::NonBinaryArgAnd.into());
            }
            children.iter().try_for_each(|child| validate_policy(child))
        }
        Clause::Or(children) => {
            if children.len() != 2 {
                return Err(CompilerError::NonBinaryArgOr.into());
            }
            children
                .iter()
                .try_for_each(|(_, child)| validate_policy(child))
        }
        Clause::Thresh(threshold) => threshold
            .iter()
            .try_for_each(|child| validate_policy(child)),
        Clause::Inscribe(inscription, child) => {
            inscription
                .validate()
                .map_err(|_| CompilerError::PolicyError(PolicyError::InvalidInscription))?;
            validate_policy(child)
        }
        Clause::Older(lock) if lock.to_consensus_u32() == 0 => {
            // Upstream exposes RelLockTime::ZERO, but Miniscript uses the
            // CSV operand as a Boolean and requires it to be nonzero.
            Err(sapio_base::timelocks::LockTimeError::InvalidPolicyLockTime(0).into())
        }
        leaf => leaf
            .is_valid()
            .map_err(|error| CompilerError::PolicyError(error).into()),
    }
}
