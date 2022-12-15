use std::{
    collections::{BTreeMap, TryReserveError},
    convert::Infallible,
    fmt::Display,
};

use schemars::schema::RootSchema;
use serde::{self, Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug)]
pub enum Error {
    Enc(serde_ipld_dagcbor::EncodeError<TryReserveError>),
    Dec(serde_ipld_dagcbor::DecodeError<Infallible>),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Enc(e) => e.fmt(f),
            Error::Dec(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for Error {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Repr(Vec<u8>);
impl ReprSpecifiable for Repr {
    fn get_repr_spec() -> ReprSpec {
        todo!()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReprSpec(Value);
impl ReprSpec {
    pub fn description(&self) -> Option<String> {
        match serde_json::from_value::<RootSchema>(self.0.clone()) {
            Err(_) => None,
            Ok(a) => a.schema.metadata.and_then(|m| m.description),
        }
    }
}
impl Display for ReprSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

pub trait ReprSpecifiable {
    fn get_repr_spec() -> ReprSpec;
}

pub fn to_string<T: Serialize>(v: &T) -> Result<String, Error> {
    serde_ipld_dagcbor::to_vec(v)
        .map_err(Error::Enc)
        .map(base64::encode)
}

pub fn from_str<'de, T: Deserialize<'static> + Clone>(s: &str) -> Result<T, Error> {
    let buf = base64::decode(s)
        .map_err(|e| Error::Dec(serde_ipld_dagcbor::DecodeError::Msg(e.to_string())))?;
    let to_clone: T = serde_ipld_dagcbor::from_slice(&buf).map_err(Error::Dec)?;
    Ok(to_clone.clone())
}

pub fn to_repr<T: Serialize>(v: &T) -> Result<Repr, Error> {
    Ok(Repr(to_bytes(v)?))
}
pub fn from_repr<'de, T: for<'a> Deserialize<'a>>(v: Repr) -> Result<T, Error> {
    from_slice(&v.0)
}

pub fn to_bytes<T: Serialize>(v: &T) -> Result<Vec<u8>, Error> {
    serde_ipld_dagcbor::to_vec(v).map_err(Error::Enc)
}

pub fn from_slice<'de, T: Deserialize<'de>>(b: &'de [u8]) -> Result<T, Error> {
    serde_ipld_dagcbor::from_slice(b).map_err(Error::Dec)
}

pub struct ValidationError(jsonschema_valid::ValidationError);
impl Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

pub fn validate(
    schema: &ReprSpec,
    data: &Repr,
) -> Result<(), Box<dyn Iterator<Item = ValidationError>>> {
    todo!()
}
