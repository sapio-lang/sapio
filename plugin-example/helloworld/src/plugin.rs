// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[deny(missing_docs)]
use sapio_wasm_plugin::client::*;
use sapio_wasm_plugin::*;

use bitcoin::util::amount::CoinAmount;
use sapio::contract::*;
use sapio::*;
use sapio_base::timelocks::RelTime;
use sapio_base::Clause;
use schemars::JsonSchema;
use serde::Deserialize;
use std::convert::{TryFrom, TryInto};

#[derive(JsonSchema, Deserialize)]
pub struct TrustlessEscrow {
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    alice_escrow_address: bitcoin::Address,
    alice_escrow_amount: CoinAmount,
    bob_escrow_address: bitcoin::Address,
    bob_escrow_amount: CoinAmount,
}

impl TrustlessEscrow {
    #[guard]
    fn cooperate(self, _ctx: Context) {
        Clause::And(vec![Clause::Key(self.alice), Clause::Key(self.bob)])
    }
    #[then]
    fn use_escrow(self, ctx: Context) {
        ctx.template()
            .add_output(
                self.alice_escrow_amount.try_into()?,
                &Compiled::from_address(self.alice_escrow_address.clone(), None),
                None,
            )?
            .add_output(
                self.bob_escrow_amount.try_into()?,
                &Compiled::from_address(self.bob_escrow_address.clone(), None),
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

REGISTER![TrustlessEscrow, "logo.png"];
