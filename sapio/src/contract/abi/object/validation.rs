// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::Object;
use bitcoin::hashes::sha256::Hash;
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
    /// The total output amount is not representable in satoshis.
    OutputAmountOverflow,
    /// The declared funding requirement is smaller than the output total.
    InsufficientAmount,
}

impl fmt::Display for ArtifactErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
            Self::OutputAmountOverflow => write!(f, "output amount total overflows"),
            Self::InsufficientAmount => {
                write!(f, "declared funding amount is below the output total")
            }
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
            if let Some(descriptor) = &object.descriptor {
                if descriptor.script_pubkey() != bitcoin::Script::from(&object.address) {
                    return Err(error(None, ArtifactErrorKind::DescriptorMismatch));
                }
            }
            for (key, template) in object.ctv_to_tx.iter().chain(&object.suggested_txs) {
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
                    if output.value != info.amount.as_sat()
                        || output.script_pubkey != bitcoin::Script::from(&info.contract.address)
                    {
                        return Err(error(ArtifactErrorKind::OutputMismatch(index)));
                    }
                    total = total
                        .checked_add(output.value)
                        .ok_or_else(|| error(ArtifactErrorKind::OutputAmountOverflow))?;
                    pending.push(&info.contract);
                }
                if total > template.max.as_sat() {
                    return Err(error(ArtifactErrorKind::InsufficientAmount));
                }
            }
        }
        Ok(())
    }
}
