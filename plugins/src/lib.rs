// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[deny(missing_docs)]
use sapio::contract::{Compilable, Context};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::Arc;

fn json_wrapped_string<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: for<'t> Deserialize<'t>,
{
    let s = String::deserialize(d)?;
    serde_json::from_str(&s).map_err(serde::de::Error::custom)
}
#[derive(Serialize, Deserialize)]
pub struct CreateArgs<S: for<'t> Deserialize<'t>>(
    /// We use json_wrapped_string to encode S to allow for a client to pass in
    /// CreateArgs without knowing the underlying type S.
    #[serde(deserialize_with = "json_wrapped_string")]
    pub S,
    pub bitcoin::Network,
    #[serde(with = "bitcoin::util::amount::serde::as_sat")] pub bitcoin::util::amount::Amount,
);

#[cfg(feature = "host")]
pub mod host;

#[cfg(feature = "client")]
pub mod client;
