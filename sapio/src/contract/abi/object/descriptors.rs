// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Wrapper for supported descriptor types
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

/// Multiple Types of Allowed Descriptor
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub enum SupportedDescriptors {
    /// # ECDSA Descriptors
    Pk(Descriptor<PublicKey>),
    /// # Taproot Descriptors
    XOnly(Descriptor<XOnlyPublicKey>),
}

impl From<Descriptor<PublicKey>> for SupportedDescriptors {
    fn from(x: Descriptor<PublicKey>) -> Self {
        SupportedDescriptors::Pk(x)
    }
}
impl From<Descriptor<XOnlyPublicKey>> for SupportedDescriptors {
    fn from(x: Descriptor<XOnlyPublicKey>) -> Self {
        SupportedDescriptors::XOnly(x)
    }
}
impl SupportedDescriptors {
    /// Regardless of descriptor type, get the output script
    pub fn script_pubkey(&self) -> Script {
        match self {
            SupportedDescriptors::Pk(p) => p.script_pubkey(),
            SupportedDescriptors::XOnly(x) => x.script_pubkey(),
        }
    }
}