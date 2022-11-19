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
impl SapioModuleBoundaryRepr {
    pub fn as_ptr(&self) -> *const u8 {
        todo!()
    }
    pub fn len(&self) -> usize {
        todo!()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SapioModuleSchema(Value);

pub trait HasSapioModuleSchema {
    fn get_schema() -> SapioModuleSchema;
}

pub fn to_string<T: Serialize>(v: &T) -> Result<String, Error> {
    todo!()
}

pub fn from_str<'de, T: Deserialize<'de>>(s: &str) -> Result<T, Error> {
    todo!()
}

pub fn to_boundary_repr<T: Serialize>(v: &T) -> Result<SapioModuleBoundaryRepr, Error> {
    todo!()
}
pub fn from_boundary_repr<'de, T: Deserialize<'de>>(
    v: SapioModuleBoundaryRepr,
) -> Result<T, Error> {
    todo!()
}

pub fn to_bytes<T: Serialize>(v: &T) -> Result<Vec<u8>, Error> {
    todo!()
}

pub fn from_slice<'de, T: Deserialize<'de>>(b: &[u8]) -> Result<T, Error> {
    todo!()
}
