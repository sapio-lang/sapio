use serde::{self, Deserialize, Serialize};
use serde_json::{Serializer, Value};

#[derive(Debug)]
pub struct Error(serde_json::Error);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}
impl std::error::Error for Error {}

struct SapioModuleBoundarySerializer<W> {
    inner: Serializer<W>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SapioModuleBoundaryRepr(Value);
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SapioModuleSchema(Value);

pub trait HasSapioModuleSchema {}

pub fn to_string<T: Serialize>(v: &T) -> Result<String, Error> {
    todo!()
}

pub fn from_str<'de, T: Deserialize<'de>>(s: &str) -> Result<T, Error> {
    todo!()
}

pub fn to_sapio_data_repr<T: Serialize>(v: &T) -> Result<SapioModuleBoundaryRepr, Error> {
    todo!()
}
pub fn from_sapio_data_repr<'de, T: Deserialize<'de>>(
    v: SapioModuleBoundaryRepr,
) -> Result<T, Error> {
    todo!()
}
