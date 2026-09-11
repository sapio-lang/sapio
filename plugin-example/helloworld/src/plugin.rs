// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Hello World Contract

#![deny(missing_docs)]
#[cfg(target_arch = "wasm32")]
use sapio_wasm_plugin::{optional_logo, REGISTER};

use sapio::contract::*;
use sapio::*;
use sapio_base::amount::CoinAmount;
use sapio_base::timelocks::RelTime;
use sapio_base::Clause;
use schemars::JsonSchema;
use serde::Deserialize;
use std::convert::{TryFrom, TryInto};

/// Trustless Escrow Contract
#[derive(JsonSchema, Deserialize)]
pub struct TrustlessEscrow {
    // TODO: Taproot Fix Encoding
    #[schemars(with = "String")]
    alice: bitcoin::XOnlyPublicKey,
    // TODO: Taproot Fix Encoding
    #[schemars(with = "String")]
    bob: bitcoin::XOnlyPublicKey,
    #[schemars(with = "String")]
    alice_escrow_address: bitcoin::Address<bitcoin::address::NetworkUnchecked>,
    alice_escrow_amount: CoinAmount,
    #[schemars(with = "String")]
    bob_escrow_address: bitcoin::Address<bitcoin::address::NetworkUnchecked>,
    bob_escrow_amount: CoinAmount,
}

impl TrustlessEscrow {
    #[guard]
    fn cooperate(self, _ctx: Context) {
        Clause::And(vec![
            Clause::Key(self.alice).into(),
            Clause::Key(self.bob).into(),
        ])
    }
    #[then]
    fn use_escrow(self, ctx: Context) {
        let network = ctx.network;
        ctx.template()
            .add_output(
                self.alice_escrow_amount.try_into()?,
                &Compiled::from_address(
                    self.alice_escrow_address.clone().require_network(network)?,
                    bitcoin::Amount::ZERO,
                ),
                None,
            )?
            .add_output(
                self.bob_escrow_amount.try_into()?,
                &Compiled::from_address(
                    self.bob_escrow_address.clone().require_network(network)?,
                    bitcoin::Amount::ZERO,
                ),
                None,
            )?
            .set_sequence(
                0,
                RelTime::try_from(std::time::Duration::from_secs(10 * 24 * 60 * 60))?.into(),
            )?
            .into()
    }
}

impl Contract for TrustlessEscrow {
    declare! {finish, Self::cooperate}
    declare! {then, Self::use_escrow}
    declare! {non updatable}
}

#[cfg(target_arch = "wasm32")]
REGISTER![TrustlessEscrow, "logo.png"];
