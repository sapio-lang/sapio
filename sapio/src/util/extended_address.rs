// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Because Addresses cannot address the case unknown or OP_RETURN we use a
//! custom type as a standin that is a superset but plays nice with existing
//! stuff.

use crate::contract::object::ObjectError;
use crate::miniscript::Descriptor;
use bitcoin::address::NetworkUnchecked;
use bitcoin::script::PushBytesBuf;
use bitcoin::{Address, ScriptBuf, XOnlyPublicKey};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;

/// A type that handles (gracefully) the fact that certain widely used
/// output types do not have an address
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(untagged)]
pub enum ExtendedAddress {
    /// A parsed address. Artifact decoding has no expected network; contract
    /// arguments must check their network before constructing a destination.
    Address(Address<NetworkUnchecked>),
    /// When we know the descriptor
    Descriptor(Descriptor<XOnlyPublicKey>),
    /// An OP_RETURN
    OpReturn(OpReturn),
    /// Unknown
    Unknown(bitcoin::ScriptBuf),
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
            bitcoin::ScriptBuf::new_op_return(
                PushBytesBuf::try_from(slice.to_vec()).expect("at most forty bytes"),
            ),
        )))
    }
}

/// Internal type for processing OpReturn through serde
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(try_from = "ScriptBuf")]
#[serde(into = "ScriptBuf")]
pub struct OpReturn(ScriptBuf);

impl TryFrom<ScriptBuf> for OpReturn {
    type Error = &'static str;
    fn try_from(s: ScriptBuf) -> std::result::Result<Self, Self::Error> {
        if s.is_op_return() {
            Ok(OpReturn(s))
        } else {
            Err("Not an Op Return")
        }
    }
}

impl From<OpReturn> for ScriptBuf {
    fn from(o: OpReturn) -> Self {
        o.0
    }
}
impl From<Address> for ExtendedAddress {
    fn from(a: Address) -> Self {
        ExtendedAddress::Address(a.into_unchecked())
    }
}
impl From<Descriptor<XOnlyPublicKey>> for ExtendedAddress {
    fn from(a: Descriptor<XOnlyPublicKey>) -> Self {
        ExtendedAddress::Descriptor(a)
    }
}

// Script bytes are network-independent; deriving them does not validate or
// promote the stored address to a network-checked contract argument.
impl From<ExtendedAddress> for ScriptBuf {
    fn from(s: ExtendedAddress) -> Self {
        match s {
            ExtendedAddress::Address(a) => a.assume_checked_ref().script_pubkey(),
            ExtendedAddress::OpReturn(OpReturn(s)) => s,
            ExtendedAddress::Unknown(s) => s,
            ExtendedAddress::Descriptor(d) => d.script_pubkey(),
        }
    }
}

impl From<&ExtendedAddress> for ScriptBuf {
    fn from(s: &ExtendedAddress) -> Self {
        match s {
            ExtendedAddress::Address(a) => a.assume_checked_ref().script_pubkey(),
            ExtendedAddress::OpReturn(OpReturn(s)) => s.clone(),
            ExtendedAddress::Unknown(s) => s.clone(),
            ExtendedAddress::Descriptor(d) => {
                let r = d.script_pubkey();
                r
            }
        }
    }
}
