// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! error types that can be returned from Sapio.
//! Where possible, concrete error types are wrapped, but in order to handle
//! errors created by the user we allow boxing an error trait.
use crate::contract::object::ObjectError;
use sapio_ctv_emulator_trait::EmulatorError;
use std::collections::LinkedList;
use std::error::Error;
use std::fmt;
/// Sapio's core error type.
#[derive(Debug)]
pub enum CompilationError {
    /// Unspecified Error -- but we should stop compiling
    TerminateCompilation,
    /// Error when a `ThenFunc` returns no Templates.
    MissingTemplates,
    /// Error if a Policy is empty
    EmptyPolicy,
    /// Error if a contract does not have sufficient funds available
    OutOfFunds,
    /// Error if a CheckSequenceVerify clause is incompatible with the sequence already set.
    /// E.g., blocks and time
    IncompatibleSequence,
    /// Error if a CheckLockTime clause is incompatible with the locktime already set.
    /// E.g., blocks and time
    IncompatibleLockTime,
    /// Error if a sequence at index j >= inputs.len() is attempted to be set
    NoSuchSequence,
    /// Error if parsing an Amount failed
    ParseAmountError(bitcoin::util::amount::ParseAmountError),
    /// Error from the Policy Compiler
    Miniscript(miniscript::policy::compiler::CompilerError),
    /// Error from the miniscript system
    MiniscriptE(miniscript::Error),
    /// Error with a Timelock
    TimeLockError(sapio_base::timelocks::LockTimeError),
    /// Error creating an object,
    CompiledObjectError(ObjectError),
    /// Failure in conditional compilation logic
    ConditionalCompilationFailed(LinkedList<String>),
    /// Unknown Error type -- either from a user or from some unhandled dependency
    Custom(Box<dyn std::error::Error>),
}
impl CompilationError {
    /// Create a custom compilation error instance
    pub fn custom<E: std::error::Error + 'static>(e: E) -> Self {
        CompilationError::Custom(Box::new(e))
    }
}

impl From<bitcoin::util::amount::ParseAmountError> for CompilationError {
    fn from(b: bitcoin::util::amount::ParseAmountError) -> Self {
        CompilationError::ParseAmountError(b)
    }
}

impl From<sapio_base::timelocks::LockTimeError> for CompilationError {
    fn from(b: sapio_base::timelocks::LockTimeError) -> Self {
        CompilationError::TimeLockError(b)
    }
}
impl From<miniscript::policy::compiler::CompilerError> for CompilationError {
    fn from(v: miniscript::policy::compiler::CompilerError) -> Self {
        CompilationError::Miniscript(v)
    }
}
impl From<miniscript::Error> for CompilationError {
    fn from(v: miniscript::Error) -> Self {
        CompilationError::MiniscriptE(v)
    }
}
impl From<ObjectError> for CompilationError {
    fn from(e: ObjectError) -> Self {
        CompilationError::CompiledObjectError(e)
    }
}

impl fmt::Display for CompilationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Error for CompilationError {}

impl From<EmulatorError> for CompilationError {
    fn from(e: EmulatorError) -> Self {
        CompilationError::Custom(Box::new(e))
    }
}
