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
    fn validate(policy: &Clause) -> Result<(), PolicyError> {
        match policy {
            Clause::And(children) => {
                if children.len() != 2 {
                    return Err(PolicyError::NonBinaryArgAnd);
                }
                children.iter().try_for_each(validate)
            }
            Clause::Or(children) => {
                if children.len() != 2 {
                    return Err(PolicyError::NonBinaryArgOr);
                }
                children.iter().try_for_each(|(_, child)| validate(child))
            }
            Clause::Threshold(required, children) => {
                if *required == 0 || *required > children.len() {
                    return Err(PolicyError::IncorrectThresh);
                }
                children.iter().try_for_each(validate)
            }
            Clause::Inscribe(inscription, child) => {
                inscription
                    .validate()
                    .map_err(|_| PolicyError::InvalidInscription)?;
                validate(child)
            }
            leaf => leaf.is_valid(),
        }
    }
    validate(policy).map_err(|error| CompilerError::PolicyError(error).into())
}
