// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! error types that can be returned from Sapio.
//! Where possible, concrete error types are wrapped, but in order to handle
//! errors created by the user we allow boxing an error trait.
use crate::contract::object::ObjectError;
use sapio_base::effects::EffectDBError;
use sapio_base::effects::EffectPath;
use sapio_base::effects::ValidFragmentError;
use sapio_base::miniscript;
use sapio_base::plugin_args::CreateArgs;
use sapio_base::simp::SIMPError;
use sapio_ctv_emulator_trait::EmulatorError;
use std::collections::LinkedList;
use std::error::Error;
use std::fmt;
type ErrT = Box<dyn std::error::Error>;
/// Sapio's core error type.
#[derive(Debug)]
pub enum CompilationError {
    /// The template passed to the compiler during a continuation has and
    /// add_guard on it, which is forbidden (since continuations should not)
    /// modify the compiled script other than to add their guards.
    AdditionalGuardsNotAllowedHere,
    /// Unspecified Error -- but we should stop compiling
    TerminateCompilation,
    /// Unspecified Error -- stop compiling, share message
    TerminateWith(String),
    /// Don't Overwrite Metadata
    OverwriteMetadata(String),
    /// Fee Specification Error
    MinFeerateError,
    /// Error when ContextPath has already been used.
    ContexPathAlreadyDerived,
    /// Error when ContextPath attempted
    InvalidPathName,
    /// Other Error for Fragment Format
    PathFragmentError(ValidFragmentError),
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
    /// Error fromt the Effects system
    EffectDBError(EffectDBError),
    /// Error in a Sapio Interactive Metadata Protocol
    SIMPError(SIMPError),
    /// Module could not be found.
    /// Used in Plugin interface (TODO: Wrap these types)
    UnknownModule,
    /// Module could not be queried
    /// Used in Plugin interface (TODO: Wrap these types)
    InvalidModule,
    /// Module failed internally
    InternalModuleError(String),
    /// Failed to get module memory
    ModuleFailedToGetMemory(ErrT),
    /// Module failed to allocate
    ModuleCouldNotAllocateError(i32, ErrT),
    /// Module failed to find function
    ModuleCouldNotFindFunction(String),
    /// Module Failed to Deallocate
    ModuleCouldNotDeallocate(i32, ErrT),
    /// Module failed to create
    ModuleCouldNotCreateContract(EffectPath, CreateArgs<serde_json::Value>, ErrT),
    /// Module failed to get_api
    ModuleCouldNotGetAPI(ErrT),
    /// Module failed to get_logo
    ModuleCouldNotGetLogo(ErrT),
    /// Module failed to get_name
    ModuleCouldNotGetName(ErrT),
    /// Module hit an error at runtime
    ModuleRuntimeError(ErrT),
    /// API Check Failed, module didn't satisfy examples.
    /// Used in Plugin interface (TODO: Wrap these types)
    ModuleFailedAPICheck(String),
    /// CompError
    ModuleCompilationErrorUnsendable(String),
    /// Issue in the Ordinals System
    OrdinalsError(String),
    /// Error while serializing
    SerializationError(serde_json::Error),
    /// Error while deserializing
    DeserializationError(serde_json::Error),
    /// No Web API enabled, but call_json was called
    WebAPIDisabled,
    /// Unknown Error type -- either from a user or from some unhandled dependency
    Custom(Box<dyn std::error::Error>),
    /// Error in continuation argument coercion
    ContinuationCoercion(String),
}

impl From<SIMPError> for CompilationError {
    fn from(e: SIMPError) -> CompilationError {
        CompilationError::SIMPError(e)
    }
}
impl From<ValidFragmentError> for CompilationError {
    fn from(e: ValidFragmentError) -> CompilationError {
        CompilationError::PathFragmentError(e)
    }
}
impl From<EffectDBError> for CompilationError {
    fn from(e: EffectDBError) -> CompilationError {
        CompilationError::EffectDBError(e)
    }
}

impl From<std::convert::Infallible> for CompilationError {
    fn from(_s: std::convert::Infallible) -> CompilationError {
        unimplemented!("Impossible, Just to make Type System Happy...");
    }
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
