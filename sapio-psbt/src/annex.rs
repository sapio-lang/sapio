//! Sapio's per-input annex declaration for signing and finalization.
//!
//! BIP371 has no dedicated annex field. This extension stores the exact
//! witness element, including its `0x50` prefix, under proprietary identifier
//! `sapio`, subtype `1`, key `annex`. Absence means no annex; an empty value is
//! invalid. Finalization must preserve these bytes as the last witness item.

use bitcoin::psbt::{raw::ProprietaryKey, Input};
use std::fmt;

/// Maximum annex size admitted by Sapio's signing protocol.
pub const MAX_ANNEX_BYTES: usize = 65_536;

/// The version-one Sapio annex field in a PSBT input map.
pub fn field() -> ProprietaryKey {
    ProprietaryKey {
        prefix: b"sapio".to_vec(),
        subtype: 1,
        key: b"annex".to_vec(),
    }
}

/// An annex declaration cannot be signed or finalized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnnexError {
    /// The witness element must begin with `0x50`.
    InvalidPrefix,
    /// The supplied annex exceeds the protocol's byte limit.
    TooLarge(usize),
    /// A finalized witness cannot acquire a new annex declaration.
    FinalizedInput,
    /// The declared annex differs from the actual finalized witness.
    FinalizedMismatch,
}

impl fmt::Display for AnnexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPrefix => formatter.write_str("annex must begin with 0x50"),
            Self::TooLarge(size) => write!(
                formatter,
                "annex has {size} bytes; maximum is {MAX_ANNEX_BYTES}"
            ),
            Self::FinalizedInput => formatter.write_str("cannot change annex of a finalized input"),
            Self::FinalizedMismatch => {
                formatter.write_str("annex declaration differs from finalized witness")
            }
        }
    }
}

impl std::error::Error for AnnexError {}

/// Validate an annex without copying it.
pub fn validate(bytes: &[u8]) -> Result<(), AnnexError> {
    if bytes.len() > MAX_ANNEX_BYTES {
        return Err(AnnexError::TooLarge(bytes.len()));
    }
    if bytes.first() != Some(&0x50) {
        return Err(AnnexError::InvalidPrefix);
    }
    Ok(())
}

/// Read the exact annex from a declaration or finalized Taproot witness.
///
/// A finalized witness is authoritative. A retained declaration must match it
/// exactly; absence of a declaration does not remove an already finalized annex.
/// Callers must separately require a Taproot previous output when an annex exists.
pub fn get(input: &Input) -> Result<Option<&[u8]>, AnnexError> {
    let declared = input
        .proprietary
        .get(&field())
        .map(|bytes| {
            validate(bytes)?;
            Ok(bytes.as_slice())
        })
        .transpose()?;
    if input.final_script_sig.is_some() || input.final_script_witness.is_some() {
        let actual = input.final_script_witness.as_ref().and_then(|witness| {
            if witness.len() >= 2 {
                witness.last().filter(|item| item.first() == Some(&0x50))
            } else {
                None
            }
        });
        if let Some(bytes) = actual {
            validate(bytes)?;
        }
        if declared.is_some() && declared != actual {
            return Err(AnnexError::FinalizedMismatch);
        }
        return Ok(actual);
    }
    Ok(declared)
}

/// Declare an annex before collecting signatures, or remove the declaration.
///
/// Changing an annex invalidates any signatures already collected for it.
pub fn set(input: &mut Input, bytes: Option<Vec<u8>>) -> Result<(), AnnexError> {
    if input.final_script_sig.is_some() || input.final_script_witness.is_some() {
        return Err(AnnexError::FinalizedInput);
    }
    if let Some(bytes) = bytes {
        validate(&bytes)?;
        input.proprietary.insert(field(), bytes);
    } else {
        input.proprietary.remove(&field());
    }
    Ok(())
}
