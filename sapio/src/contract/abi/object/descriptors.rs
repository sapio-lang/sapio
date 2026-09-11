// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Wrapper for supported descriptor types

use super::RawTaproot;
pub use crate::contract::abi::studio::*;
use bitcoin::PublicKey;
use bitcoin::ScriptBuf;
use bitcoin::XOnlyPublicKey;
use miniscript::*;
use sapio_base::miniscript;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Multiple Types of Allowed Descriptor
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub enum SupportedDescriptors {
    /// # ECDSA Descriptors
    Pk(Descriptor<PublicKey>),
    /// # Taproot Descriptors
    XOnly(Descriptor<XOnlyPublicKey>),
    /// # Checked raw Taproot scripts
    /// Spending data for scripts whose witness requirements are not described
    /// by Miniscript. This representation carries no satisfaction-weight bound.
    Taproot(RawTaproot),
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
impl From<RawTaproot> for SupportedDescriptors {
    fn from(tree: RawTaproot) -> Self {
        SupportedDescriptors::Taproot(tree)
    }
}
impl SupportedDescriptors {
    /// Regardless of descriptor type, get the output script
    pub fn script_pubkey(&self) -> ScriptBuf {
        match self {
            SupportedDescriptors::Pk(p) => p.script_pubkey(),
            SupportedDescriptors::XOnly(x) => x.script_pubkey(),
            SupportedDescriptors::Taproot(tree) => tree.script_pubkey(),
        }
    }
}
