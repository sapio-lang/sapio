// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! A finite payout tree with a cooperative exit and explicit per-transaction fees.
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{Address, Amount, XOnlyPublicKey};
use sapio::contract::*;
use sapio::*;
use sapio_base::Clause;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};

/// One miner's payout, in integer satoshis, to an internal Taproot key.
#[derive(JsonSchema, Serialize, Deserialize, Clone)]
pub struct PoolShare {
    #[serde(with = "bitcoin::util::amount::serde::as_sat")]
    #[schemars(with = "u64")]
    pub amount: Amount,
    #[schemars(with = "String")]
    pub key: XOnlyPublicKey,
}

/// A payout tree whose funding includes every leaf and every tree fee.
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct MiningPayout {
    pub participants: Vec<PoolShare>,
    /// Maximum children per transaction; must be at least two.
    pub radix: usize,
    #[serde(with = "bitcoin::util::amount::serde::as_sat")]
    #[schemars(with = "u64")]
    pub fee_sats_per_tx: Amount,
}

fn invalid(message: &str) -> CompilationError {
    CompilationError::TerminateWith(message.into())
}

fn transaction_count(participants: usize, radix: usize) -> Result<u64, CompilationError> {
    if participants == 0 || radix < 2 {
        return Err(invalid(
            "A payout needs participants and a radix of at least two",
        ));
    }
    // Each non-root grouping reduces the queue by radix - 1. One final
    // transaction pays the remaining children, including a singleton payout.
    u64::try_from(participants.saturating_sub(2) / (radix - 1) + 1)
        .map_err(|_| invalid("Payout tree size overflow"))
}

impl MiningPayout {
    /// Allocate the complete reward after fees. Canonical key order determines
    /// who receives each remainder satoshi, independently of input ordering.
    pub fn from_reward(
        mut keys: Vec<XOnlyPublicKey>,
        reward: Amount,
        radix: usize,
        fee_sats_per_tx: Amount,
    ) -> Result<Self, CompilationError> {
        let count = transaction_count(keys.len(), radix)?;
        keys.sort_unstable();
        if keys.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(invalid("A miner key may appear only once"));
        }
        let fees = fee_sats_per_tx
            .as_sat()
            .checked_mul(count)
            .ok_or_else(|| invalid("Payout fees overflow"))?;
        let available = reward
            .as_sat()
            .checked_sub(fees)
            .ok_or(CompilationError::OutOfFunds)?;
        let miners = u64::try_from(keys.len()).map_err(|_| invalid("Too many miners"))?;
        if available < miners {
            return Err(CompilationError::OutOfFunds);
        }
        let each = available / miners;
        let remainder = available % miners;
        let participants = keys
            .into_iter()
            .enumerate()
            .map(|(index, key)| PoolShare {
                key,
                amount: Amount::from_sat(each + u64::from((index as u64) < remainder)),
            })
            .collect();
        Ok(Self {
            participants,
            radix,
            fee_sats_per_tx,
        })
    }

    /// Exact funding for this tree, including all descendants' reserved fees.
    pub fn funding_required(&self) -> Result<Amount, CompilationError> {
        let count = transaction_count(self.participants.len(), self.radix)?;
        let mut keys = BTreeSet::new();
        let mut total = self
            .fee_sats_per_tx
            .as_sat()
            .checked_mul(count)
            .ok_or_else(|| invalid("Payout fees overflow"))?;
        for participant in &self.participants {
            if participant.amount == Amount::ZERO || !keys.insert(participant.key) {
                return Err(invalid(
                    "Payouts need positive amounts and distinct miner keys",
                ));
            }
            total = total
                .checked_add(participant.amount.as_sat())
                .ok_or_else(|| invalid("Payout amount overflow"))?;
        }
        Ok(Amount::from_sat(total))
    }

    #[guard]
    fn cooperate(self, _ctx: Context) {
        let keys: Vec<_> = self
            .participants
            .iter()
            .map(|p| Clause::Key(p.key))
            .collect();
        Clause::Threshold(keys.len(), keys)
    }

    #[then]
    fn expand(self, ctx: Context) {
        if self.funding_required()? > ctx.funds() {
            return Err(CompilationError::OutOfFunds);
        }
        let mut ctx = ctx;
        let mut counter = 0u64;
        let mut next_context = || {
            counter += 1;
            ctx.derive_num(counter)
        };
        let mut queue: VecDeque<(Amount, Box<dyn PayThisThing>)> = self
            .participants
            .iter()
            .map(|payment| {
                let leaf: Box<dyn PayThisThing> = Box::new(JustAKey::new(payment, next_context()?));
                Ok((payment.amount, leaf))
            })
            .collect::<Result<_, CompilationError>>()?;
        loop {
            let children: Vec<_> = queue.drain(..self.radix.min(queue.len())).collect();
            if queue.is_empty() {
                let mut builder = next_context()?.template();
                for (amount, contract) in &children {
                    builder = builder.add_output(*amount, contract.as_compilable(), None)?;
                }
                return builder.add_fees(self.fee_sats_per_tx)?.into();
            }
            let bundle = Box::new(PayoutBundle {
                contracts: children,
                fees: self.fee_sats_per_tx,
            });
            queue.push_back((bundle.total_to_pay()?, bundle));
        }
    }
}

impl Contract for MiningPayout {
    declare! {then, Self::expand}
    declare! {finish, Self::cooperate}
    declare! {non updatable}
}

trait PayThisThing {
    fn keys(&self) -> Vec<XOnlyPublicKey>;
    fn as_compilable(&self) -> &dyn Compilable;
}

struct JustAKey(XOnlyPublicKey, Compiled);
impl JustAKey {
    fn new(payment: &PoolShare, ctx: Context) -> Self {
        let address = Address::p2tr(
            &Secp256k1::verification_only(),
            payment.key,
            None,
            ctx.network,
        );
        Self(payment.key, Compiled::from_address(address, payment.amount))
    }
}
impl PayThisThing for JustAKey {
    fn keys(&self) -> Vec<XOnlyPublicKey> {
        vec![self.0]
    }
    fn as_compilable(&self) -> &dyn Compilable {
        &self.1
    }
}

struct PayoutBundle {
    contracts: Vec<(Amount, Box<dyn PayThisThing>)>,
    fees: Amount,
}
impl PayThisThing for PayoutBundle {
    fn keys(&self) -> Vec<XOnlyPublicKey> {
        self.contracts
            .iter()
            .flat_map(|(_, contract)| contract.keys())
            .collect()
    }
    fn as_compilable(&self) -> &dyn Compilable {
        self
    }
}
impl PayoutBundle {
    fn total_to_pay(&self) -> Result<Amount, CompilationError> {
        self.contracts
            .iter()
            .try_fold(self.fees.as_sat(), |sum, (amount, _)| {
                sum.checked_add(amount.as_sat())
                    .ok_or_else(|| invalid("Payout bundle overflow"))
            })
            .map(Amount::from_sat)
    }
    #[guard]
    fn cooperate(self, _ctx: Context) {
        let keys: Vec<_> = self.keys().into_iter().map(Clause::Key).collect();
        Clause::Threshold(keys.len(), keys)
    }
    #[then]
    fn expand(self, ctx: Context) {
        let mut builder = ctx.template();
        for (amount, contract) in &self.contracts {
            builder = builder.add_output(*amount, contract.as_compilable(), None)?;
        }
        builder.add_fees(self.fees)?.into()
    }
}
impl Contract for PayoutBundle {
    declare! {then, Self::expand}
    declare! {finish, Self::cooperate}
    declare! {non updatable}
}

#[cfg(test)]
mod tests;
