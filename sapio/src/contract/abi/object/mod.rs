// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Object is the output of Sapio Compilation & can be linked to a specific coin

pub mod error;
pub use error::*;
pub mod bind;
pub mod descriptors;
pub use descriptors::*;

use crate::contract::abi::continuation::ContinuationPoint;
pub use crate::contract::abi::studio::*;
use crate::template::Template;
use crate::util::amountrange::AmountRange;
use crate::util::extended_address::ExtendedAddress;
use ::miniscript::*;
use bitcoin::hashes::sha256;

use bitcoin::util::amount::Amount;

use sapio_base::effects::EffectPath;
use sapio_base::effects::PathFragment;
use sapio_base::serialization_helpers::SArc;

use sapio_base::simp::SIMPError;
use sapio_base::simp::SIMP;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use std::sync::Arc;
/// Metadata for Object, arbitrary KV set.
#[derive(Serialize, Deserialize, Clone, JsonSchema, Debug, PartialEq, Eq, Default)]
pub struct ObjectMetadata {
    /// Additional non-standard fields for future upgrades
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
    /// SIMP: Sapio Interactive Metadata Protocol
    pub simp: BTreeMap<i64, serde_json::Value>,
}
impl ObjectMetadata {
    /// Is there any metadata in this field?
    pub fn is_empty(&self) -> bool {
        *self == Default::default()
    }

    /// attempts to add a SIMP to the object meta.
    ///
    /// Returns [`SIMPError::AlreadyDefined`] if one was previously set.
    pub fn add_simp<S: SIMP>(mut self, s: S) -> Result<Self, SIMPError> {
        let old = self
            .simp
            .insert(S::get_protocol_number(), serde_json::to_value(&s)?);
        if let Some(old) = old {
            Err(SIMPError::AlreadyDefined(old))
        } else {
            Ok(self)
        }
    }
}

/// Object holds a contract's complete context required post-compilation
/// There is no guarantee that Object is properly constructed presently.
//TODO: Make type immutable and correct by construction...
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Object {
    /// a map of template hashes to the corresponding template, that in the
    /// policy are a CTV protected
    #[serde(
        rename = "template_hash_to_template_map",
        skip_serializing_if = "BTreeMap::is_empty",
        default
    )]
    pub ctv_to_tx: BTreeMap<sha256::Hash, Template>,
    /// a map of template hashes to the corresponding template, that in the
    /// policy are not necessarily CTV protected but we might want to know about
    /// anyways.
    #[serde(
        rename = "suggested_template_hash_to_template_map",
        skip_serializing_if = "BTreeMap::is_empty",
        default
    )]
    pub suggested_txs: BTreeMap<sha256::Hash, Template>,
    /// A Map of arguments to continue execution and generate an update at this
    /// point via a passed message
    #[serde(
        rename = "continuation_points",
        skip_serializing_if = "BTreeMap::is_empty",
        default
    )]
    pub continue_apis: BTreeMap<SArc<EffectPath>, ContinuationPoint>,
    /// The base location for the set of continue_apis.
    pub root_path: SArc<EffectPath>,
    /// The Object's address, or a Script if no address is possible
    pub address: ExtendedAddress,
    /// The Object's descriptor -- if there is one known/available
    #[serde(
        rename = "known_descriptor",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub descriptor: Option<SupportedDescriptors>,
    /// The amount_range safe to send this object
    pub amount_range: AmountRange,
    /// metadata generated for this contract
    pub metadata: ObjectMetadata,
}

impl Object {
    /// Creates an object from a given address. The optional AmountRange argument determines the
    /// safe bounds the contract can receive, otherwise it is set to any.
    pub fn from_address(address: bitcoin::Address, a: Option<AmountRange>) -> Object {
        Object {
            ctv_to_tx: BTreeMap::new(),
            suggested_txs: BTreeMap::new(),
            continue_apis: Default::default(),
            root_path: SArc(EffectPath::push(
                None,
                PathFragment::Named(SArc(Arc::new("".into()))),
            )),
            address: address.into(),
            descriptor: None,
            amount_range: a.unwrap_or_else(|| {
                let mut a = AmountRange::new();
                a.update_range(Amount::min_value());
                a.update_range(Amount::from_sat(21_000_000 * 100_000_000));
                a
            }),
            metadata: Default::default(),
        }
    }

    /// Creates an object from a given script. The optional AmountRange argument determines the
    /// safe bounds the contract can receive, otherwise it is set to any.
    pub fn from_script(
        script: bitcoin::Script,
        a: Option<AmountRange>,
        net: bitcoin::Network,
    ) -> Result<Object, ObjectError> {
        bitcoin::Address::from_script(&script, net)
            .ok_or_else(|| ObjectError::UnknownScriptType(script.clone()))
            .map(|m| Object::from_address(m, a))
    }
    /// create an op_return of no more than 40 bytes
    pub fn from_op_return<'a, I: ?Sized>(data: &'a I) -> Result<Object, ObjectError>
    where
        &'a [u8]: From<&'a I>,
    {
        Ok(Object {
            ctv_to_tx: BTreeMap::new(),
            suggested_txs: BTreeMap::new(),
            continue_apis: Default::default(),
            root_path: SArc(EffectPath::push(
                None,
                PathFragment::Named(SArc(Arc::new("".into()))),
            )),
            address: ExtendedAddress::make_op_return(data)?,
            descriptor: None,
            amount_range: AmountRange::new(),
            metadata: Default::default(),
        })
    }

    /// converts a descriptor and an optional AmountRange to a Object object.
    /// This can be used for e.g. creating raw SegWit Scripts.
    pub fn from_descriptor<T>(d: Descriptor<T>, a: Option<AmountRange>) -> Self
    where
        Descriptor<T>: Into<SupportedDescriptors>,
        T: MiniscriptKey + ToPublicKey,
    {
        Object {
            ctv_to_tx: BTreeMap::new(),
            suggested_txs: BTreeMap::new(),
            continue_apis: Default::default(),
            root_path: SArc(EffectPath::push(
                None,
                PathFragment::Named(SArc(Arc::new("".into()))),
            )),
            address: d.address(bitcoin::Network::Bitcoin).unwrap().into(),
            descriptor: Some(d.into()),
            amount_range: a.unwrap_or_else(|| {
                let mut a = AmountRange::new();
                a.update_range(Amount::min_value());
                a.update_range(Amount::from_sat(21_000_000 * 100_000_000));
                a
            }),
            metadata: Default::default(),
        }
    }
}
