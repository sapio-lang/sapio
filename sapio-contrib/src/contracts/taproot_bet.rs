// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Historical Taproot activation bet, with a finite recurring payout schedule.
//! Cancellation is available after its timeout regardless of activation status.
use bitcoin::{Amount, Script};
use sapio::contract::*;
use sapio::*;
use sapio_base::timelocks::AnyRelTimeLock;
use sapio_macros::guard;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Taproot Recurring Bet.
/// This data structure captures all the arguments required to build a contract.
#[derive(JsonSchema, Serialize, Deserialize, Clone)]
pub struct TapBet {
    /// How much Bitcoin to release per period
    #[schemars(with = "f64")]
    #[serde(with = "bitcoin::util::amount::serde::as_btc")]
    pub amount_per_time: Amount,
    /// How much in fees to pay per cycle.
    /// Zero is allowed; each continuation still releases a positive payout.
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

impl TapBet {
    fn validate(&self, ctx: &Context) -> Result<(), CompilationError> {
        let same_units = matches!(
            (self.period, self.cancel_timeout),
            (AnyRelTimeLock::RH(_), AnyRelTimeLock::RH(_))
                | (AnyRelTimeLock::RT(_), AnyRelTimeLock::RT(_))
        );
        if self.amount_per_time.as_sat() == 0
            || self.period.get() & 0xffff == 0
            || !same_units
            || self.cancel_timeout.get() <= self.period.get()
            || !self.taproot_script.is_v1_p2tr()
            || bitcoin::XOnlyPublicKey::from_slice(&self.taproot_script.as_bytes()[2..]).is_err()
            || !self.cancel_to.is_valid_for_network(ctx.network)
            || ctx.funds() <= self.fees_per_time
        {
            return Err(CompilationError::TerminateWith(
                "Invalid TapBet payout, timeout, destination, or funding".into(),
            ));
        }
        Ok(())
    }

    #[guard]
    fn period_over(self, _ctx: Context) {
        self.period.into()
    }

    #[then(guarded_by = "[Self::period_over]")]
    fn continue_expansion(self, ctx: Context) {
        let spendable = ctx.funds() - self.fees_per_time;
        let payout = std::cmp::min(self.amount_per_time, spendable);
        let remainder = spendable - payout;
        let destination = Compiled::from_script(self.taproot_script.clone(), None, ctx.network)?;
        let mut builder = ctx
            .template()
            .set_label("continue_expansion".into())
            .set_sequence(0, self.period)?
            .add_output(payout, &destination, None)?;
        if remainder > self.fees_per_time {
            builder = builder.add_output(remainder, self, None)?;
        } else if remainder.as_sat() > 0 {
            // A remainder unable to fund another fee-bearing step goes back now.
            builder = builder.add_output(
                remainder,
                &Compiled::from_address(self.cancel_to.clone(), None),
                None,
            )?;
        }
        builder.add_fees(self.fees_per_time)?.into()
    }

    #[guard]
    fn timeout(self, _ctx: Context) {
        self.cancel_timeout.into()
    }

    #[then(guarded_by = "[Self::timeout]")]
    fn stop_expansion(self, ctx: Context) {
        let payout = ctx.funds() - self.fees_per_time;
        ctx.template()
            .set_label("stop_expansion".into())
            .set_sequence(0, self.cancel_timeout)?
            .add_output(
                payout,
                &Compiled::from_address(self.cancel_to.clone(), None),
                None,
            )?
            .add_fees(self.fees_per_time)?
            .into()
    }
}

impl Contract for TapBet {
    fn ensure_amount(&self, ctx: Context) -> Result<Amount, CompilationError> {
        self.validate(&ctx)?;
        Ok(ctx.funds())
    }
    declare! {then, Self::stop_expansion, Self::continue_expansion}
    declare! {non updatable}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{address, context};
    use sapio_base::timelocks::{RelHeight, RelTime};
    fn bet() -> TapBet {
        TapBet {
            amount_per_time: Amount::from_sat(1000),
            fees_per_time: Amount::from_sat(100),
            period: RelHeight::from(1).into(),
            cancel_timeout: RelHeight::from(2).into(),
            taproot_script: address(1).script_pubkey(),
            cancel_to: address(2),
        }
    }
    #[test]
    fn recurrence_pays_fees_and_terminates_with_remainder() {
        let bet = bet();
        let mut compiled = bet.compile(context(2350)).unwrap();
        for (funds, payout, remainder) in [(2350, 1000, 1250), (1250, 1000, 150), (150, 50, 0)] {
            assert_eq!(compiled.ctv_to_tx.len(), 2);
            let next = compiled
                .ctv_to_tx
                .values()
                .find(|t| t.tx.input[0].sequence == 1)
                .unwrap();
            let cancel = compiled
                .ctv_to_tx
                .values()
                .find(|t| t.tx.input[0].sequence == 2)
                .unwrap();
            assert_eq!(cancel.tx.output[0].value, funds - 100);
            assert_eq!(
                cancel.tx.output[0].script_pubkey,
                address(2).script_pubkey()
            );
            assert_eq!(next.tx.output[0].value, payout);
            assert_eq!(next.tx.output[0].script_pubkey, address(1).script_pubkey());
            assert_eq!(
                next.tx.output.iter().map(|o| o.value).sum::<u64>(),
                funds - 100
            );
            assert_eq!(next.max.as_sat(), funds);
            if remainder == 0 {
                assert_eq!(next.tx.output.len(), 1);
                break;
            }
            assert_eq!(next.tx.output[1].value, remainder);
            compiled = next.outputs[1].contract.clone();
        }
        let final_step = bet.compile(context(1150)).unwrap();
        let next = final_step
            .ctv_to_tx
            .values()
            .find(|t| t.tx.input[0].sequence == 1)
            .unwrap();
        assert_eq!(next.tx.output[1].value, 50);
        assert_eq!(next.tx.output[1].script_pubkey, address(2).script_pubkey());
    }
    #[test]
    fn zero_fees_progress_and_invalid_contracts_fail() {
        let mut v = bet();
        v.fees_per_time = Amount::from_sat(0);
        assert!(v.compile(context(2000)).is_ok());
        let mut v = bet();
        v.amount_per_time = Amount::from_sat(0);
        assert!(v.compile(context(2000)).is_err());
        let mut v = bet();
        v.cancel_timeout = v.period;
        assert!(v.compile(context(2000)).is_err());
        let mut v = bet();
        v.cancel_timeout = RelTime::from(2).into();
        assert!(v.compile(context(2000)).is_err());
        let mut v = bet();
        v.period = RelHeight::from(0).into();
        assert!(v.compile(context(2000)).is_err());
        let mut v = bet();
        v.taproot_script = Script::new();
        assert!(v.compile(context(2000)).is_err());
        let mut v = bet();
        v.taproot_script = bitcoin::blockdata::script::Builder::new()
            .push_int(1)
            .push_slice(&[0xff; 32])
            .into_script();
        assert!(v.compile(context(2000)).is_err());
        assert!(bet().compile(context(100)).is_err());
        let mut v = bet();
        v.cancel_to.network = bitcoin::Network::Bitcoin;
        assert!(v.compile(context(2000)).is_err());
        let mut v = bet();
        let mut signet = context(2000);
        signet.network = bitcoin::Network::Signet;
        v.cancel_to.network = bitcoin::Network::Testnet;
        assert!(v.compile(signet).is_ok());
        let json = serde_json::to_value(bet()).unwrap();
        assert!(serde_json::from_value::<TapBet>(json)
            .unwrap()
            .compile(context(2000))
            .is_ok());
    }
}
