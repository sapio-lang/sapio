use sapio_ctv_emulator_trait::EmulatorError;
use std::error::Error;
use std::fmt;
#[derive(Debug)]
pub enum CompilationError {
    TerminateCompilation,
    MissingTemplates,
    EmptyPolicy,
    OutOfFunds,
    IncompatibleSequence,
    IncompatibleLockTime,
    NoSuchSequence,
    ParseAmountError(bitcoin::util::amount::ParseAmountError),
    Miniscript(miniscript::policy::compiler::CompilerError),
    MiniscriptE(miniscript::Error),
    TimeLockError(sapio_base::timelocks::LockTimeError),
    Custom(Box<dyn std::error::Error>),
}
impl CompilationError {
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
