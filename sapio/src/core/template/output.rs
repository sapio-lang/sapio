use super::*;
/// Metadata for outputs, arbitrary KV set.
pub type OutputMeta = HashMap<String, String>;

/// An Output is not a literal Bitcoin Output, but contains data needed to construct one, and
/// metadata for linking & ABI building
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Output {
    #[serde(with = "bitcoin::util::amount::serde::as_sat")]
    #[schemars(with = "i64")]
    pub amount: Amount,
    pub contract: crate::contract::Compiled,
    pub metadata: OutputMeta,
}
