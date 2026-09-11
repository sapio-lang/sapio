// Copyright Judica, Inc 2026
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Policy interchange between contract authors, custom compilers and Sapio.
//!
//! Custom compilers describe spending conditions; Sapio still combines those
//! conditions with action guards and transaction commitments. Raw fragments
//! retain their instruction order and multiplicity when combined.

use crate::{Clause, Ctv, Emulatable};
use bitcoin::blockdata::opcodes::{all, Class, ClassifyContext};
use bitcoin::blockdata::script::{Error as ScriptError, Instruction};
use bitcoin::ScriptBuf;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

/// Immutable policy source accepted by Sapio's script lowering stage.
///
/// Miniscript clauses remain source policies until their enclosing branch is
/// assembled. Adjacent native predicates compile as one run, so a timelock
/// or hashlock can share its run's authorization or transaction commitment.
/// Raw fragments separate runs; each native run must independently pass the
/// Miniscript compiler's safety checks.
/// Validation and resource limits still apply during lowering, including to
/// policies produced by a custom [`PolicyCompiler`].
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum ScriptPolicy {
    /// The existing native Miniscript policy language.
    Miniscript(Clause),
    /// A CTV predicate explicitly resolved with public lowering inputs.
    Emulatable(Emulatable<Ctv>),
    /// A checked raw fragment with backend-defined witness requirements.
    Script(ScriptFragment),
    /// Require every child, in encounter order. An empty conjunction is true.
    And(Vec<ScriptPolicy>),
    /// Permit any child. An empty disjunction has no spending alternative.
    Or(Vec<ScriptPolicy>),
}

impl From<Clause> for ScriptPolicy {
    fn from(policy: Clause) -> Self {
        Self::Miniscript(policy)
    }
}

impl From<ScriptFragment> for ScriptPolicy {
    fn from(script: ScriptFragment) -> Self {
        Self::Script(script)
    }
}

impl From<Emulatable<Ctv>> for ScriptPolicy {
    fn from(predicate: Emulatable<Ctv>) -> Self {
        Self::Emulatable(predicate)
    }
}

/// Translate another policy language into Sapio's immutable policy source.
///
/// Implementations must preserve the source language's authorization and
/// produce raw fragments satisfying [`ScriptFragment`]'s witness contract.
/// Sapio validates fragment boundaries and controls policy expansion; it does
/// not prove an arbitrary backend's authorization, satisfiability or witness
/// size. Returning a raw fragment does not provide a satisfaction-weight bound
/// or an automatic witness finalizer.
pub trait PolicyCompiler {
    /// Translate this policy without constructing a contract artifact directly.
    fn compile_policy(&self) -> Result<ScriptPolicy, PolicyError>;
}

impl PolicyCompiler for Clause {
    fn compile_policy(&self) -> Result<ScriptPolicy, PolicyError> {
        Ok(ScriptPolicy::Miniscript(self.clone()))
    }
}

impl PolicyCompiler for Emulatable<Ctv> {
    fn compile_policy(&self) -> Result<ScriptPolicy, PolicyError> {
        Ok(ScriptPolicy::Emulatable(*self))
    }
}

impl PolicyCompiler for ScriptPolicy {
    fn compile_policy(&self) -> Result<ScriptPolicy, PolicyError> {
        Ok(self.clone())
    }
}

impl PolicyCompiler for ScriptFragment {
    fn compile_policy(&self) -> Result<ScriptPolicy, PolicyError> {
        Ok(ScriptPolicy::Script(self.clone()))
    }
}

impl<P: PolicyCompiler, E: fmt::Display> PolicyCompiler for Result<P, E> {
    fn compile_policy(&self) -> Result<ScriptPolicy, PolicyError> {
        match self {
            Ok(policy) => policy.compile_policy(),
            Err(error) => Err(PolicyError::Backend(error.to_string())),
        }
    }
}

/// Raw tapscript whose control flow and alternative stack are locally scoped.
///
/// A backend must arrange its witness so the fragment leaves its truth value
/// on top of the main stack. Sapio may append `OP_VERIFY` and another fragment.
/// Main-stack consumption, clean-stack satisfaction and signature requirements
/// remain the backend's responsibility; this constructor does not infer them.
///
/// Validation prevents control flow, early-success opcodes or alternative-stack
/// effects from escaping into adjacent fragments. Code separators are excluded
/// because the current signing path assumes none. No witness-size bound is
/// implied. Deserialization performs the same validation as construction.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct ScriptFragment(ScriptBuf);

impl ScriptFragment {
    /// Check the fragment's instruction and composition boundaries.
    pub fn new(script: ScriptBuf) -> Result<Self, PolicyError> {
        validate_tapscript(&script)?;
        Ok(Self(script))
    }

    /// Borrow the checked script without exposing mutation.
    pub fn as_script(&self) -> &bitcoin::Script {
        &self.0
    }

    /// Consume the fragment and recover its script.
    pub fn into_script(self) -> ScriptBuf {
        self.0
    }
}

impl TryFrom<ScriptBuf> for ScriptFragment {
    type Error = PolicyError;

    fn try_from(script: ScriptBuf) -> Result<Self, Self::Error> {
        Self::new(script)
    }
}

impl<'de> Deserialize<'de> for ScriptFragment {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(ScriptBuf::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// A source-policy or raw-script boundary error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyError {
    /// The script contains a truncated or otherwise undecodable instruction.
    MalformedScript {
        /// Byte offset of the instruction.
        offset: usize,
        /// The underlying instruction-decoding error.
        error: ScriptError,
    },
    /// An instruction can bypass composition, invalidate signing, or is illegal.
    ForbiddenOpcode {
        /// Byte offset of the opcode.
        offset: usize,
        /// The rejected opcode byte.
        opcode: u8,
    },
    /// An `ELSE` or `ENDIF` has no corresponding conditional.
    UnexpectedConditional {
        /// Byte offset of the opcode.
        offset: usize,
        /// The unmatched opcode byte.
        opcode: u8,
    },
    /// An `IF` or `NOTIF` remains open at the end of the fragment.
    UnclosedConditional {
        /// Byte offset of the opening opcode.
        offset: usize,
    },
    /// A path consumes alternative-stack state belonging to another fragment.
    AltStackUnderflow {
        /// Byte offset of the `FROMALTSTACK` instruction.
        offset: usize,
    },
    /// Alternative-stack depths differ at a branch merge or are nonzero at EOF.
    AltStackImbalance {
        /// Byte offset of the merge, or the script length for an EOF error.
        offset: usize,
    },
    /// A custom policy compiler could not translate its source language.
    Backend(String),
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedScript { offset, error } => {
                write!(f, "invalid script instruction at byte {offset}: {error}")
            }
            Self::ForbiddenOpcode { offset, opcode } => {
                write!(f, "unsupported opcode 0x{opcode:02x} at byte {offset}")
            }
            Self::UnexpectedConditional { offset, opcode } => {
                write!(
                    f,
                    "unmatched conditional opcode 0x{opcode:02x} at byte {offset}"
                )
            }
            Self::UnclosedConditional { offset } => {
                write!(f, "unclosed conditional starting at byte {offset}")
            }
            Self::AltStackUnderflow { offset } => {
                write!(f, "alternative-stack underflow at byte {offset}")
            }
            Self::AltStackImbalance { offset } => {
                write!(f, "unbalanced alternative stack at byte {offset}")
            }
            Self::Backend(message) => write!(f, "policy compiler: {message}"),
        }
    }
}

impl std::error::Error for PolicyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::MalformedScript { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// Validate boundaries shared by raw fragments and assembled tapscript leaves.
///
/// Checks instruction decoding, forbidden opcodes, balanced conditionals and
/// locally balanced alternative-stack effects. Both arms of a conditional must
/// merge at the same alternative-stack depth; without `ELSE`, the untaken arm
/// retains the entry depth. Pushed bytes are data, including opcode byte values.
/// This is a composition check, not proof of consensus validity or satisfaction.
pub fn validate_tapscript(script: &bitcoin::Script) -> Result<(), PolicyError> {
    struct Conditional {
        offset: usize,
        other_alt_depth: usize,
    }

    let mut conditionals: Vec<Conditional> = vec![];
    let mut alt_depth = 0usize;
    let mut offset = 0usize;
    for instruction in script.instructions() {
        let instruction =
            instruction.map_err(|error| PolicyError::MalformedScript { offset, error })?;
        match instruction {
            Instruction::PushBytes(data) => {
                let prefix = match script.as_bytes()[offset] {
                    0x4c => 2,
                    0x4d => 3,
                    0x4e => 5,
                    _ => 1,
                };
                offset += prefix + data.len();
                continue;
            }
            Instruction::Op(opcode) => {
                if matches!(
                    opcode.classify(ClassifyContext::TapScript),
                    Class::SuccessOp | Class::IllegalOp
                ) || opcode == all::OP_CODESEPARATOR
                {
                    return Err(PolicyError::ForbiddenOpcode {
                        offset,
                        opcode: opcode.to_u8(),
                    });
                }
                match opcode {
                    all::OP_IF | all::OP_NOTIF => conditionals.push(Conditional {
                        offset,
                        other_alt_depth: alt_depth,
                    }),
                    all::OP_ELSE => {
                        let conditional =
                            conditionals
                                .last_mut()
                                .ok_or(PolicyError::UnexpectedConditional {
                                    offset,
                                    opcode: opcode.to_u8(),
                                })?;
                        // Repeated ELSE is legal: switch back to the saved depth
                        // of the other execution path each time.
                        std::mem::swap(&mut alt_depth, &mut conditional.other_alt_depth);
                    }
                    all::OP_ENDIF => {
                        let conditional =
                            conditionals
                                .pop()
                                .ok_or(PolicyError::UnexpectedConditional {
                                    offset,
                                    opcode: opcode.to_u8(),
                                })?;
                        if alt_depth != conditional.other_alt_depth {
                            return Err(PolicyError::AltStackImbalance { offset });
                        }
                    }
                    all::OP_TOALTSTACK => alt_depth += 1,
                    all::OP_FROMALTSTACK => {
                        alt_depth = alt_depth
                            .checked_sub(1)
                            .ok_or(PolicyError::AltStackUnderflow { offset })?;
                    }
                    _ => {}
                }
            }
        }
        offset += 1;
    }
    if let Some(conditional) = conditionals.first() {
        return Err(PolicyError::UnclosedConditional {
            offset: conditional.offset,
        });
    }
    if alt_depth != 0 {
        return Err(PolicyError::AltStackImbalance { offset });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(bytes: &[u8]) -> ScriptBuf {
        ScriptBuf::from(bytes.to_vec())
    }

    #[test]
    fn rejects_every_success_opcode_even_inside_an_untaken_branch() {
        let success = [80, 98]
            .into_iter()
            .chain(126..=129)
            .chain(131..=134)
            .chain(137..=138)
            .chain(141..=142)
            .chain(149..=153)
            .chain(187..=254);
        for opcode in success {
            assert_eq!(
                ScriptFragment::new(script(&[0x00, 0x63, opcode, 0x68, 0x51])),
                Err(PolicyError::ForbiddenOpcode { offset: 2, opcode }),
            );
        }
        assert_eq!(
            ScriptFragment::new(script(&[0x51, 0xab])),
            Err(PolicyError::ForbiddenOpcode {
                offset: 1,
                opcode: 0xab
            }),
        );
    }

    #[test]
    fn pushed_opcode_bytes_are_data_and_instruction_offsets_are_exact() {
        for opcode in [0x50, 0x63, 0x67, 0x68, 0x6c, 0xab, 0xff] {
            assert!(ScriptFragment::new(script(&[1, opcode, 0x75, 0x51])).is_ok());
        }
        // Nonminimal PUSHDATA is decoded without treating its contents as code.
        assert_eq!(
            ScriptFragment::new(script(&[0x4c, 1, 0x50, 0x75, 0xab])),
            Err(PolicyError::ForbiddenOpcode {
                offset: 4,
                opcode: 0xab
            }),
        );
        assert_eq!(
            ScriptFragment::new(script(&[0x51, 0x4d, 1])),
            Err(PolicyError::MalformedScript {
                offset: 1,
                error: ScriptError::EarlyEndOfScript
            }),
        );
    }

    #[test]
    fn conditionals_cannot_capture_or_escape_adjacent_fragments() {
        for opcode in [0x67, 0x68] {
            assert_eq!(
                ScriptFragment::new(script(&[opcode])),
                Err(PolicyError::UnexpectedConditional { offset: 0, opcode }),
            );
        }
        for opcode in [0x63, 0x64] {
            assert_eq!(
                ScriptFragment::new(script(&[0x51, opcode, 0x51])),
                Err(PolicyError::UnclosedConditional { offset: 1 }),
            );
        }
        assert!(ScriptFragment::new(script(&[
            0x63, 0x64, 0x51, 0x67, 0x00, 0x68, 0x67, 0x51, 0x68
        ]))
        .is_ok());
    }

    #[test]
    fn alternative_stack_effects_must_balance_on_both_execution_paths() {
        assert_eq!(
            ScriptFragment::new(script(&[0x6c])),
            Err(PolicyError::AltStackUnderflow { offset: 0 }),
        );
        assert_eq!(
            ScriptFragment::new(script(&[0x6b])),
            Err(PolicyError::AltStackImbalance { offset: 1 }),
        );
        // The implicit false arm does not push onto the alternative stack.
        assert_eq!(
            ScriptFragment::new(script(&[0x63, 0x6b, 0x68, 0x6c])),
            Err(PolicyError::AltStackImbalance { offset: 2 }),
        );
        // Both branches add one item; the common suffix removes that item.
        assert!(ScriptFragment::new(script(&[0x63, 0x6b, 0x67, 0x6b, 0x68, 0x6c])).is_ok());
        // Repeated ELSE resumes the accumulated state of the matching arm.
        assert!(ScriptFragment::new(script(&[0x63, 0x6b, 0x67, 0x67, 0x6c, 0x68, 0x51])).is_ok());
        // A nested branch cannot consume an item that exists only in its sibling.
        assert_eq!(
            ScriptFragment::new(script(&[0x63, 0x6b, 0x67, 0x63, 0x6c, 0x68, 0x68])),
            Err(PolicyError::AltStackUnderflow { offset: 4 }),
        );
    }

    #[test]
    fn fragment_serialization_and_schema_match_the_checked_script_format() {
        let fragment = ScriptFragment::new(script(&[0x51])).unwrap();
        assert_eq!(
            serde_json::to_value(&fragment).unwrap(),
            serde_json::json!("51")
        );
        assert_eq!(
            serde_json::from_str::<ScriptFragment>("\"51\"").unwrap(),
            fragment
        );
        for invalid in ["\"50\"", "\"ab\"", "\"63\"", "\"4d01\"", "\"6c\""] {
            assert!(serde_json::from_str::<ScriptFragment>(invalid).is_err());
        }
        let schema = schemars::schema_for!(ScriptFragment);
        assert_eq!(schema.as_value()["type"], "string");
    }

    #[test]
    fn policy_interchange_preserves_source_and_validates_nested_fragments() {
        // Compiling this alone as a safe Miniscript would fail; translation
        // retains it so the enclosing branch can supply authorization.
        let clause = Clause::Older(miniscript::RelLockTime::from_height(10));
        assert_eq!(
            clause.compile_policy().unwrap(),
            ScriptPolicy::Miniscript(clause.clone())
        );
        let policy = ScriptPolicy::And(vec![
            clause.into(),
            ScriptFragment::new(script(&[0x51])).unwrap().into(),
        ]);
        let encoded = serde_json::to_value(&policy).unwrap();
        assert_eq!(
            serde_json::from_value::<ScriptPolicy>(encoded).unwrap(),
            policy
        );
        assert_eq!(policy.compile_policy().unwrap(), policy);
        let error = serde_json::from_value::<ScriptPolicy>(serde_json::json!({
            "And": [{"Miniscript": "older(10)"}, {"Script": "50"}]
        }))
        .unwrap_err();
        assert!(error.to_string().contains("unsupported opcode 0x50"));
    }
}
