use std::error::Error;
use std::fmt;
#[derive(Debug)]
pub enum CompilationError {
    TerminateCompilation,
    MissingTemplates,
    EmptyPolicy,
    OutOfFunds,
    ParseAmountError(bitcoin::util::amount::ParseAmountError),
    Miniscript(miniscript::policy::compiler::CompilerError),
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
impl From<miniscript::policy::compiler::CompilerError> for CompilationError {
    fn from(v: miniscript::policy::compiler::CompilerError) -> Self {
        CompilationError::Miniscript(v)
    }
}

impl fmt::Display for CompilationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Error for CompilationError {}
