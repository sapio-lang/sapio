// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
use bitcoin::Address;
use bitcoin::PublicKey;
use sapio::contract::*;
use sapio::util::amountrange::*;
use sapio::*;
use sapio_base::Clause;
use schemars::*;
use serde::*;
use std::collections::VecDeque;
/// A payment to a specific address
#[derive(JsonSchema, Serialize, Deserialize, Clone)]
pub struct PoolShare {
    /// The amount to send
    #[serde(with = "bitcoin::util::amount::serde::as_btc")]
    #[schemars(with = "f64")]
    pub amount: bitcoin::util::amount::Amount,
    /// # Address
    /// The Address to send to
    pub key: bitcoin::PublicKey,
}
/// Documentation placed here will be visible to users!
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct MiningPayout {
    /// all of the payments needing to be sent
    pub participants: Vec<PoolShare>,
    /// the radix of the tree to build. Optimal for users should be around 4 or
    /// 5 (with CTV, not emulators).
    pub radix: usize,
    #[serde(with = "bitcoin::util::amount::serde::as_sat")]
    #[schemars(with = "u64")]
    pub fee_sats_per_tx: bitcoin::util::amount::Amount,
}

use bitcoin::util::amount::Amount;
trait CoopKeys {
    fn get_keys(&self) -> Vec<PublicKey>;
}
trait PayThisThing: CoopKeys {
    fn as_compilable(&self) -> &dyn Compilable;
}

struct JustAKey(PublicKey, Box<dyn Compilable>);
impl CoopKeys for JustAKey {
    fn get_keys(&self) -> Vec<PublicKey> {
        vec![self.0.clone()]
    }
}
impl JustAKey {
    fn new(payment: &PoolShare, ctx: &Context) -> Result<Self, CompilationError> {
        let mut amt = AmountRange::new();
        amt.update_range(payment.amount);
        let address = Address::p2wpkh(&payment.key, ctx.network)
            .map_err(|_| CompilationError::TerminateCompilation)?;
        let b: Box<dyn Compilable> = Box::new(Compiled::from_address(address, Some(amt)));
        Ok(JustAKey(payment.key, b))
    }
}
impl PayThisThing for JustAKey {
    fn as_compilable(&self) -> &dyn Compilable {
        self.1.as_ref()
    }
}
struct PayoutBundle {
    contracts: Vec<(Amount, Box<dyn PayThisThing>)>,
    fees: Amount,
}
impl CoopKeys for PayoutBundle {
    fn get_keys(&self) -> Vec<PublicKey> {
        let mut v = vec![];
        for c in self.contracts.iter() {
            v.append(&mut c.1.get_keys());
        }
        v
    }
}
impl PayThisThing for PayoutBundle {
    fn as_compilable(&self) -> &dyn Compilable {
        self
    }
}
impl PayoutBundle {
    guard! {
        fn cooperate(self, _ctx) {
           let v : Vec<_>= self.get_keys().into_iter().map(Clause::Key).collect();
           Clause::Threshold(v.len(), v)
        }
    }
    then! {
        fn expand(self, ctx) {
            let mut bld = ctx.template();
            for (amt, ct) in self.contracts.iter() {
                bld = bld.add_output(*amt, ct.as_compilable(), None)?;
            }
            bld.add_fees(self.fees)?.into()
        }
    }

    fn total_to_pay(&self) -> Amount {
        let mut amt = self.fees;
        for (x, _) in self.contracts.iter() {
            amt += *x;
        }
        amt
    }
}
impl Contract for PayoutBundle {
    declare! {then, Self::expand}
    declare! {finish, Self::cooperate}
    declare! {non updatable}
}
impl MiningPayout {
    guard! {
        fn cooperate(self, _ctx) {
           let v : Vec<_>= self.participants.iter().map(|x|Clause::Key(x.key.clone())).collect();
           Clause::Threshold(v.len(), v)
        }
    }
    then! {
        fn expand(self, ctx) {

            let mut queue : VecDeque<(Amount, Box<dyn PayThisThing>)> = self.participants.iter().map(|payment| {
                let b: Box<dyn PayThisThing> = Box::new(JustAKey::new(payment, ctx)?);
                Ok((payment.amount, b))
            }).collect::<Result<VecDeque<_>, CompilationError>>()?;

            loop {
                let v : Vec<_> = queue.drain(0..std::cmp::min(self.radix, queue.len())).collect();
                if queue.len() == 0 {
                    let mut builder = ctx.template();
                    for pay in v.iter() {
                        builder = builder.add_output(pay.0, pay.1.as_compilable(), None)?;
                    }
                    builder =builder.add_fees(self.fee_sats_per_tx)?;
                    return builder.into();
                } else {
                    let pay = Box::new(PayoutBundle{contracts:v, fees: self.fee_sats_per_tx});
                    queue.push_back((pay.total_to_pay(), pay))
                }
            }
    }}
}
impl Contract for MiningPayout {
    declare! {then, Self::expand}
    declare! {finish, Self::cooperate}
    declare! {non updatable}
}
