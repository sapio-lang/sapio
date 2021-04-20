// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Because Addresses cannot address the case unknown or OP_RETURN we use a
//! custom type as a standin that is a superset but plays nice with existing
//! stuff.

use crate::contract::object::ObjectError;
use bitcoin::{Address, Script};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;

/// A type that handles (gracefully) the fact that certain widely used
/// output types do not have an address
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
#[serde(untagged)]
pub enum ExtendedAddress {
    /// A regular standard address type
    Address(bitcoin::Address),
    /// An OP_RETURN
    OpReturn(OpReturn),
    /// Unknown
    Unknown(bitcoin::Script),
}
impl ExtendedAddress {
    /// create an OP_RETURN address type
    pub fn make_op_return<'a, I: ?Sized>(data: &'a I) -> Result<Self, ObjectError>
    where
        &'a [u8]: From<&'a I>,
    {
        let slice: &[u8] = data.into();
        if slice.len() > 40 {
            return Err(ObjectError::OpReturnTooLong);
        }
        Ok(ExtendedAddress::OpReturn(OpReturn(
            bitcoin::Script::new_op_return(slice),
        )))
    }
}

/// Internal type for processing OpReturn through serde
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
#[serde(try_from = "Script")]
#[serde(into = "Script")]
pub struct OpReturn(Script);

impl TryFrom<Script> for OpReturn {
    type Error = &'static str;
    fn try_from(s: Script) -> std::result::Result<Self, Self::Error> {
        if s.is_op_return() {
            Ok(OpReturn(s))
        } else {
            Err("Not an Op Return")
        }
    }
}

impl From<OpReturn> for Script {
    fn from(o: OpReturn) -> Self {
        o.0
    }
}
impl From<Address> for ExtendedAddress {
    fn from(a: Address) -> Self {
        ExtendedAddress::Address(a)
    }
}

impl From<ExtendedAddress> for Script {
    fn from(s: ExtendedAddress) -> Self {
        match s {
            ExtendedAddress::Address(a) => a.script_pubkey(),
            ExtendedAddress::OpReturn(OpReturn(s)) => s,
            ExtendedAddress::Unknown(s) => s,
        }
    }
}
