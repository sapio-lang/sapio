use super::*;
/// Metadata for outputs, arbitrary KV set.
pub type OutputMeta = HashMap<String, String>;

/// An Output is not a literal Bitcoin Output, but contains data needed to construct one, and
/// metadata for linking & ABI building
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Output {
    pub amount: CoinAmount,
    pub contract: crate::contract::Compiled,
    pub metadata: OutputMeta,
}
impl Output {
    /// Creates a new Output, forcing the compilation of the compilable object and defaulting
    /// metadata if not provided to blank.
    pub fn new<T: crate::contract::Compilable>(
        amount: CoinAmount,
        contract: &T,
        metadata: Option<OutputMeta>,
    ) -> Result<Output, CompilationError> {
        Ok(Output {
            amount,
            contract: contract.compile()?,
            metadata: metadata.unwrap_or_else(HashMap::new),
        })
    }
}
