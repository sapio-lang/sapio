// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[deny(missing_docs)]
use bitcoin::Amount;
use bitcoin::Script;
use sapio::contract::*;
use sapio::contract::*;
use sapio::template::Template;
use sapio::util::amountrange::AmountRange;
use sapio::*;
use sapio::*;
use sapio_base::timelocks::AnyRelTimeLock;
use sapio_base::Clause;
use sapio_wasm_plugin::client::*;
use sapio_wasm_plugin::*;
use schemars::*;
use schemars::*;
use serde::*;
use serde::*;
use std::convert::TryInto;
use std::marker::PhantomData;

/// Taproot Recurring Bet.
/// This data structure captures all the arguments required to build a contract.
#[derive(JsonSchema, Serialize, Deserialize, Clone)]
pub struct TapBet {
    /// How much Bitcoin to release per period
    #[schemars(with = "f64")]
    #[serde(with = "bitcoin::util::amount::serde::as_btc")]
    pub amount_per_time: Amount,
    /// How much in fees to pay per cycle.
    /// TODO: In theory, this could be zero, as miners could manually add such
    /// transactions (which they topet a reward out of) to their mempools.
    /// TODO: Optional, make cancellation path have a different feerate
    #[schemars(with = "f64")]
    #[serde(with = "bitcoin::util::amount::serde::as_btc")]
    pub fees_per_time: Amount,
    /// How frequently should we test to see if Taproot is active?
    pub period: AnyRelTimeLock,
    /// How long to wait to allow early-abort of the contract unfolding (should
    /// be > period)
    pub cancel_timeout: AnyRelTimeLock,
    /// An externally generated Taproot script (not address) to send the funds to
    pub taproot_script: Script,
    /// An arbitrary bitcoin address to send the funds to on cancellation
    pub cancel_to: bitcoin::Address,
}

/// This defines the interface for the TapBet Contract
impl Contract for TapBet {
    /// The "next steps" that can happen for an instance of a TapBet
    /// is either to:
    /// - stop_expansion: return the funds safely to the creator because Taproot is active
    /// - continue_expansion: take amount_per_time of the funds and send them to a taproot address.
    ///     > If taproot is active, the funds are safe in that key
    ///     > If taproot is not active, a miner may steal the funds
    declare! {then, Self::stop_expansion, Self::continue_expansion}
    /// you can ignore this line, it is only needed for an advanced Sapio feature
    /// and will be able to be removed when a specific rust feature stablizes.
    declare! {non updatable}
}

/// The actual logic for each TapBet
impl TapBet {
    /// The waiting period is over, sample if Taproot is active
    guard! {period_over |s, ctx| { s.period.into() }}
    then! {continue_expansion [Self::period_over] |s, ctx| {
        // creates a new transaction template for the next step
        // of this contract
        let mut builder = ctx.template().set_label("continue_expansion".into());
        // set the sequence validly
        builder = builder.set_sequence(0, s.period.into())?;
        // if we have sufficient funds, pay out to a taproot address now
        if builder.ctx().funds() >= s.amount_per_time {
            let mut range = AmountRange::new();
            range.update_range(s.amount_per_time);
            builder = builder.add_output(
                s.amount_per_time,
                &Compiled::from_script(s.taproot_script.clone(), Some(range), ctx.network)?,
                None
            )?;
        }
        // if we have funds remaining, make a recursive TapBet with the same
        // parameters.
        if builder.ctx().funds() >= s.fees_per_time {
            let amt =
                builder.ctx().funds() - s.fees_per_time;
            if amt > Amount::from_sat(0) {
                builder = builder.add_output(
                    amt,
                    s,
                    None
                )?;
            }
        }
        builder.into()
    }}

    /// The timeout period is over
    guard! {timeout |s, ctx| { s.cancel_timeout.into() }}
    then! {stop_expansion [Self::timeout] |s, ctx| {
        let mut builder  = ctx.template().set_label("stop_expansion".into());
        builder = builder.set_sequence(0, s.cancel_timeout.into())?;
        // Pay out to the orginal owner
        if builder.ctx().funds() >= s.fees_per_time {
            let amt = builder.ctx().funds() - s.fees_per_time;
            if amt > Amount::from_sat(0) {
                builder = builder.add_output(
                    amt,
                    &Compiled::from_address(s.cancel_to.clone(), None),
                    None
                )?;
            }
        }
        builder.into()
    }}
}
