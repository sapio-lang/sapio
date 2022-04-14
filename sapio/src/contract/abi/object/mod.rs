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

pub use crate::contract::abi::studio::*;
use crate::contract::abi::continuation::ContinuationPoint;
use crate::template::Template;
use crate::util::amountrange::AmountRange;
use crate::util::extended_address::ExtendedAddress;
use ::miniscript::{self, *};
use bitcoin::hashes::sha256;
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::util::amount::Amount;
use bitcoin::util::psbt::PartiallySignedTransaction;
use bitcoin::util::taproot::TaprootBuilder;
use bitcoin::util::taproot::TaprootSpendInfo;
use bitcoin::OutPoint;
use bitcoin::PublicKey;
use bitcoin::Script;
use bitcoin::XOnlyPublicKey;
use sapio_base::effects::EffectPath;
use sapio_base::effects::PathFragment;
use sapio_base::serialization_helpers::SArc;
use sapio_base::txindex::TxIndex;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// Object holds a contract's complete context required post-compilation
/// There is no guarantee that Object is properly constructed presently.
//TODO: Make type immutable and correct by construction...
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Object {
    /// a map of template hashes to the corresponding template, that in the
    /// policy are a CTV protected
    #[serde(
        rename = "template_hash_to_template_map",
        skip_serializing_if = "HashMap::is_empty",
        default
    )]
    pub ctv_to_tx: HashMap<sha256::Hash, Template>,
    /// a map of template hashes to the corresponding template, that in the
    /// policy are not necessarily CTV protected but we might want to know about
    /// anyways.
    #[serde(
        rename = "suggested_template_hash_to_template_map",
        skip_serializing_if = "HashMap::is_empty",
        default
    )]
    pub suggested_txs: HashMap<sha256::Hash, Template>,
    /// A Map of arguments to continue execution and generate an update at this
    /// point via a passed message
    #[serde(
        rename = "continuation_points",
        skip_serializing_if = "HashMap::is_empty",
        default
    )]
    pub continue_apis: HashMap<SArc<EffectPath>, ContinuationPoint>,
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
}

impl Object {
    /// Creates an object from a given address. The optional AmountRange argument determines the
    /// safe bounds the contract can receive, otherwise it is set to any.
    pub fn from_address(address: bitcoin::Address, a: Option<AmountRange>) -> Object {
        Object {
            ctv_to_tx: HashMap::new(),
            suggested_txs: HashMap::new(),
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
            ctv_to_tx: HashMap::new(),
            suggested_txs: HashMap::new(),
            continue_apis: Default::default(),
            root_path: SArc(EffectPath::push(
                None,
                PathFragment::Named(SArc(Arc::new("".into()))),
            )),
            address: ExtendedAddress::make_op_return(data)?,
            descriptor: None,
            amount_range: AmountRange::new(),
        })
    }
}