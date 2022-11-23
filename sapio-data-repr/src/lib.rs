use std::fmt::Display;

use schemars::schema::RootSchema;
use serde::{self, Deserialize, Serialize};
use serde_json::{Serializer, Value};

#[derive(Debug)]
pub struct Error(serde_json::Error);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Error {}

struct SapioModuleBoundarySerializer<W> {
    inner: Serializer<W>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SapioModuleBoundaryRepr(Value);
impl HasSapioModuleSchema for SapioModuleBoundaryRepr {
    fn get_schema() -> SapioModuleSchema {
        todo!()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SapioModuleSchema(Value);
impl SapioModuleSchema {
    pub fn description(&self) -> Option<String> {
        match serde_json::from_value::<RootSchema>(self.0.clone()) {
            Err(_) => None,
            Ok(a) => a.schema.metadata.and_then(|m| m.description),
        }
    }
}
impl Display for SapioModuleSchema {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

pub trait HasSapioModuleSchema {
    fn get_schema() -> SapioModuleSchema;
}

pub fn to_string<T: Serialize>(v: &T) -> Result<String, Error> {
    serde_json::to_string(v).map_err(Error)
}

pub fn from_str<'de, T: Deserialize<'de>>(s: &'de str) -> Result<T, Error> {
    serde_json::from_str(s).map_err(Error)
}

pub fn to_boundary_repr<T: Serialize>(v: &T) -> Result<SapioModuleBoundaryRepr, Error> {
    serde_json::to_value(v)
        .map(SapioModuleBoundaryRepr)
        .map_err(Error)
}
pub fn from_boundary_repr<'de, T: for<'a> Deserialize<'a>>(
    v: SapioModuleBoundaryRepr,
) -> Result<T, Error> {
    serde_json::from_value(v.0).map_err(Error)
}

pub fn to_bytes<T: Serialize>(v: &T) -> Result<Vec<u8>, Error> {
    serde_json::to_vec(v).map_err(Error)
}

pub fn from_slice<'de, T: Deserialize<'de>>(b: &'de [u8]) -> Result<T, Error> {
    serde_json::from_slice(b).map_err(Error)
}

pub struct ValidationError(jsonschema_valid::ValidationError);
impl Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

pub fn validate(
    schema: &SapioModuleSchema,
    data: &SapioModuleBoundaryRepr,
) -> Result<(), Box<dyn Iterator<Item = ValidationError>>> {
    let cfg = jsonschema_valid::Config::from_schema(
        &schema.0,
        Some(jsonschema_valid::schemas::Draft::Draft6),
    )
    .unwrap();
    let validation_errs = cfg.validate(&data.0);
    match validation_errs {
        Ok(()) => Ok(()),
        Err(e) => Err(Box::new(
            e.map(ValidationError)
                .collect::<Vec<ValidationError>>()
                .into_iter(),
        )),
    }
}
