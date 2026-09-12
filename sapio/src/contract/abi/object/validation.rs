// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::Object;
use bitcoin::hashes::sha256::Hash;
use bitcoin::Amount;
use sapio_base::effects::EffectPath;
use sapio_base::serialization_helpers::SArc;
use sapio_base::util::CTVHash;
use std::fmt;

/// The location and cause of an inconsistent compiled artifact.
#[derive(Debug)]
pub struct ArtifactError {
    /// The affected contract's compilation path.
    pub path: SArc<EffectPath>,
    /// The template map key, when the error concerns a template.
    pub template: Option<Hash>,
    /// The violated invariant.
    pub kind: ArtifactErrorKind,
}

/// Invariants required to bind a compiled artifact to transaction inputs.
#[derive(Debug, PartialEq, Eq)]
pub enum ArtifactErrorKind {
    /// A retained program policy does not match its descriptor or resource limits.
    InvalidProgramPolicy(String),
    /// Every committed template must record the injected covenant policy.
    MissingCovenant,
    /// Public lowering inputs cannot be used to resolve covenant predicates.
    InvalidCovenantLowering(String),
    /// The descriptor and advertised address describe different scripts.
    DescriptorMismatch,
    /// Binding requires a contract input at index zero.
    MissingInput,
    /// Only index zero is supported by the binder.
    UnsupportedInputIndex(u32),
    /// Input metadata must describe every transaction input exactly once.
    InputMetadataCount,
    /// Output metadata must describe every transaction output exactly once.
    OutputMetadataCount,
    /// A template must be unsigned, with empty scriptSigs and witnesses.
    SignedInput(usize),
    /// The map key, cached hash and transaction commitment must agree.
    TemplateHashMismatch,
    /// An output's amount or receiving script differs from its metadata.
    OutputMismatch(usize),
    /// A parent output cannot fund every declared transaction of its child.
    UnderfundedChild {
        /// The parent transaction output funding the child.
        index: usize,
        /// The amount committed to the parent output.
        available: Amount,
        /// The child's largest declared input-zero requirement.
        required: Amount,
    },
    /// The total output amount is not representable in satoshis.
    OutputAmountOverflow,
    /// The declared funding requirement is smaller than the output total.
    InsufficientAmount,
    /// Input zero's requirement exceeds the aggregate, or a single-input
    /// transaction claims funding from nonexistent auxiliary inputs.
    InvalidInputAmount,
    /// An object's input requirement is below one of its declared templates.
    InvalidInputRequirement,
}

impl fmt::Display for ArtifactErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProgramPolicy(reason) => write!(f, "invalid program policy: {reason}"),
            Self::MissingCovenant => write!(f, "committed template has no covenant policy"),
            Self::InvalidCovenantLowering(reason) => {
                write!(f, "invalid covenant lowering: {reason}")
            }
            Self::DescriptorMismatch => write!(f, "descriptor does not match the address script"),
            Self::MissingInput => write!(f, "missing contract input zero"),
            Self::UnsupportedInputIndex(index) => {
                write!(f, "unsupported contract input index {index}; expected zero")
            }
            Self::InputMetadataCount => {
                write!(f, "input metadata count does not match the transaction")
            }
            Self::OutputMetadataCount => {
                write!(f, "output metadata count does not match the transaction")
            }
            Self::SignedInput(index) => {
                write!(f, "template input {index} has a scriptSig or witness")
            }
            Self::TemplateHashMismatch => write!(
                f,
                "template map key, cached hash and transaction commitment disagree"
            ),
            Self::OutputMismatch(index) => write!(
                f,
                "output {index} amount or script does not match its metadata"
            ),
            Self::UnderfundedChild {
                index,
                available,
                required,
            } => write!(
                f,
                "output {index} provides {} sat but its child requires {} sat",
                available.to_sat(),
                required.to_sat()
            ),
            Self::OutputAmountOverflow => write!(f, "output amount total overflows"),
            Self::InsufficientAmount => {
                write!(f, "declared funding amount is below the output total")
            }
            Self::InvalidInputAmount => {
                write!(
                    f,
                    "contract input funding requirement is inconsistent with the transaction"
                )
            }
            Self::InvalidInputRequirement => write!(
                f,
                "contract input requirement is below a declared template requirement"
            ),
        }
    }
}

impl fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid artifact at {}",
            String::from(self.path.0.as_ref().clone())
        )?;
        if let Some(template) = self.template {
            write!(f, ", template {template}")?;
        }
        write!(f, ": {}", self.kind)
    }
}

impl std::error::Error for ArtifactError {}

impl Object {
    /// Validate the entire graph before binding or requesting funding.
    ///
    /// Checks structural consistency and transaction commitments, including
    /// suggested transactions. This does not prove policy satisfaction, chain
    /// enforcement, funding availability, or the advertised fee rate.
    pub fn validate(&self) -> Result<(), ArtifactError> {
        let mut pending = vec![self];
        while let Some(object) = pending.pop() {
            let error = |template, kind| ArtifactError {
                path: object.root_path.clone(),
                template,
                kind,
            };
            object
                .covenant_requirements
                .lowering
                .validate()
                .map_err(|e| {
                    error(
                        None,
                        ArtifactErrorKind::InvalidCovenantLowering(e.to_string()),
                    )
                })?;
            if let Some(descriptor) = &object.descriptor {
                if descriptor.script_pubkey() != bitcoin::ScriptBuf::from(&object.address) {
                    return Err(error(None, ArtifactErrorKind::DescriptorMismatch));
                }
            }
            object
                .validate_program_policies()
                .map_err(|kind| error(None, kind))?;
            for (committed, key, template) in object
                .ctv_to_tx
                .iter()
                .map(|(key, template)| (true, key, template))
                .chain(
                    object
                        .suggested_txs
                        .iter()
                        .map(|(key, template)| (false, key, template)),
                )
            {
                let error = |kind| error(Some(*key), kind);
                let tx = &template.tx;
                if tx.input.is_empty() {
                    return Err(error(ArtifactErrorKind::MissingInput));
                }
                if template.ctv_index != 0 {
                    return Err(error(ArtifactErrorKind::UnsupportedInputIndex(
                        template.ctv_index,
                    )));
                }
                if tx.input.len() != template.inputs.len() {
                    return Err(error(ArtifactErrorKind::InputMetadataCount));
                }
                if tx.output.len() != template.outputs.len() {
                    return Err(error(ArtifactErrorKind::OutputMetadataCount));
                }
                for (index, input) in tx.input.iter().enumerate() {
                    if !input.script_sig.is_empty() || !input.witness.is_empty() {
                        return Err(error(ArtifactErrorKind::SignedInput(index)));
                    }
                }
                if *key != template.ctv || template.ctv != tx.get_ctv_hash(0) {
                    return Err(error(ArtifactErrorKind::TemplateHashMismatch));
                }
                let mut total = 0u64;
                for (index, (output, info)) in tx.output.iter().zip(&template.outputs).enumerate() {
                    if output.value != info.amount
                        || output.script_pubkey != bitcoin::ScriptBuf::from(&info.contract.address)
                    {
                        return Err(error(ArtifactErrorKind::OutputMismatch(index)));
                    }
                    let required = info.contract.required_input_amount;
                    if info.amount < required {
                        return Err(error(ArtifactErrorKind::UnderfundedChild {
                            index,
                            available: info.amount,
                            required,
                        }));
                    }
                    total = total
                        .checked_add(output.value.to_sat())
                        .ok_or_else(|| error(ArtifactErrorKind::OutputAmountOverflow))?;
                    pending.push(&info.contract);
                }
                if total > template.max.to_sat() {
                    return Err(error(ArtifactErrorKind::InsufficientAmount));
                }
                if template.required_input_amount > template.max
                    || (tx.input.len() == 1 && template.required_input_amount != template.max)
                {
                    return Err(error(ArtifactErrorKind::InvalidInputAmount));
                }
                if object.required_input_amount < template.required_input_amount {
                    return Err(error(ArtifactErrorKind::InvalidInputRequirement));
                }
                if committed
                    && !object
                        .covenant_requirements
                        .predicates
                        .contains(&sapio_base::covenant::Ctv(*key))
                {
                    return Err(error(ArtifactErrorKind::MissingCovenant));
                }
            }
        }
        Ok(())
    }
}
