// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Payment Pool Contract for Sapio Studio Advent Calendar Entry
#[deny(missing_docs)]
use crate::sapio_base::Clause;
use bitcoin::hashes::hex::FromHex;
use bitcoin::hashes::sha256;
use bitcoin::hashes::Hash;
use bitcoin::secp256k1::Message;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::secp256k1::Signature;
use bitcoin::util::amount::Amount;
use bitcoin::Address;
use bitcoin::PublicKey;
use sapio::contract::actions::conditional_compile::ConditionalCompileType;
use sapio::contract::*;
use sapio::util::amountrange::{AmountF64, AmountU64};
use sapio::*;
use sapio_contrib::contracts::treepay::{Payment, TreePay};
use sapio_wasm_plugin::client::*;
use sapio_wasm_plugin::*;
use schemars::*;
use serde::*;
use std::collections::{BTreeMap, HashMap};

use std::convert::TryInto;
use std::io::Write;

#[derive(Deserialize, JsonSchema, Clone)]
struct PaymentPool {
    /// # Pool Members
    /// map of all initial balances as PK to BTC
    members: BTreeMap<PublicKey, AmountF64>,
    /// The current sequence number (for authenticating state updates)
    sequence: u64,
    /// If to require signatures or not (debugging, should be true)
    sig_needed: bool,
}

impl Contract for PaymentPool {
    declare! {then, Self::ejection}
    declare! {updatable<DoTx>, Self::do_tx}
}
/// Payment Request
#[derive(Deserialize, JsonSchema, Serialize)]
struct PaymentRequest {
    /// # Signature
    /// hex encoded signature of the fee, sequence number, and payments
    hex_der_sig: String,
    /// # Fees
    /// Fees for this participant to pay in Satoshis
    fee: AmountU64,
    /// # Payments
    /// Mapping of Address to Bitcoin Amount (btc)
    payments: BTreeMap<Address, AmountF64>,
}
/// New Update message for generating a transaction from.
#[derive(Deserialize, JsonSchema, Serialize)]
struct DoTx {
    /// # Payments
    /// A mapping of public key in members to signed list of payouts with a fee rate.
    payments: HashMap<PublicKey, PaymentRequest>,
}
/// required...
impl Default for DoTx {
    fn default() -> Self {
        DoTx {
            payments: HashMap::new(),
        }
    }
}
impl StatefulArgumentsTrait for DoTx {}

/// helper for rust type system issue
fn default_coerce(
    k: <PaymentPool as Contract>::StatefulArguments,
) -> Result<DoTx, CompilationError> {
    Ok(k)
}

impl PaymentPool {
    /// Sum Up all the balances
    fn total(&self) -> Amount {
        self.members
            .values()
            .cloned()
            .map(Amount::from)
            .fold(Amount::from_sat(0), |a, b| a + b)
    }
    /// Only compile an ejection if the pool has other users in it, otherwise
    /// it's base case.
    #[compile_if]
    fn has_eject(self, _ctx: Context) {
        if self.members.len() > 1 {
            ConditionalCompileType::Required
        } else {
            ConditionalCompileType::Never
        }
    }
    /// Split the pool in two -- users can eject multiple times to fully eject.
    #[then(compile_if = "[Self::has_eject]")]
    fn ejection(self, ctx: Context) {
        let t = ctx.template();
        let mid = (self.members.len() + 1) / 2;
        // find the middle
        let key = self.members.keys().nth(mid).expect("must be present");
        let mut pool_one: PaymentPool = self.clone();
        pool_one.sequence += 1;
        let pool_two = PaymentPool {
            // removes the back half including key
            members: pool_one.members.split_off(&key),
            sequence: self.sequence + 1,
            sig_needed: self.sig_needed,
        };
        let amt_one = pool_one.total();
        let amt_two = pool_two.total();
        t.add_output(amt_one, &pool_one, None)?
            .add_output(amt_two, &pool_two, None)?
            .into()
    }

    /// all signed the transaction!
    #[guard]
    fn all_signed(self, _ctx: Context) {
        Clause::Threshold(
            self.members.len(),
            self.members.keys().cloned().map(Clause::Key).collect(),
        )
    }
    /// This Function will create a proposed transaction that is safe to sign
    /// given a list of data from participants.
    #[continuation(
        web_api,
        guarded_by = "[Self::all_signed]",
        coerce_args = "default_coerce"
    )]
    fn do_tx(self, ctx: Context, update: DoTx) {
        let _effects = unsafe { ctx.get_effects_internal() };
        // don't allow empty updates.
        if update.payments.is_empty() {
            return empty();
        }
        // collect members with updated balances here
        let mut new_members = self.members.clone();
        // verification context
        let secp = Secp256k1::new();
        // collect all the payments
        let mut all_payments = vec![];
        let mut spent = Amount::from_sat(0);
        // for each payment...
        for (
            from,
            PaymentRequest {
                hex_der_sig,
                fee,
                payments,
            },
        ) in update.payments.iter()
        {
            // every from must be in the members
            let balance = self
                .members
                .get(from)
                .ok_or(CompilationError::TerminateCompilation)?;
            let new_balance = Amount::from(*balance)
                - (payments
                    .values()
                    .cloned()
                    .map(Amount::from)
                    .fold(Amount::from_sat(0), |a, b| a + b)
                    + Amount::from(*fee));
            // check for no underflow
            if new_balance.as_sat() < 0 {
                return Err(CompilationError::TerminateCompilation);
            }
            // updates the balance or remove if empty
            if new_balance.as_sat() > 0 {
                new_members.insert(from.clone(), new_balance.into());
            } else {
                new_members.remove(from);
            }

            // collect all the payment
            for (address, amt) in payments.iter() {
                spent += Amount::from(*amt);
                all_payments.push(Payment {
                    address: address.clone(),
                    amount: Amount::from(*amt).into(),
                })
            }
            // Check the signature for this request
            // came from this user
            if self.sig_needed {
                let mut hasher = sha256::Hash::engine();
                hasher.write(&self.sequence.to_le_bytes());
                hasher.write(&Amount::from(*fee).as_sat().to_le_bytes());
                for (address, amt) in payments.iter() {
                    hasher.write(&Amount::from(*amt).as_sat().to_le_bytes());
                    hasher.write(address.script_pubkey().as_bytes());
                }
                let h = sha256::Hash::from_engine(hasher);
                let m = Message::from_slice(&h.as_inner()[..]).expect("Correct Size");
                let signed: Vec<u8> = FromHex::from_hex(&hex_der_sig)
                    .map_err(|_| CompilationError::TerminateCompilation)?;
                let sig = Signature::from_der(&signed)
                    .map_err(|_| CompilationError::TerminateCompilation)?;
                let _: () = secp
                    .verify(&m, &sig, &from.inner)
                    .map_err(|_| CompilationError::TerminateCompilation)?;
            }
        }
        // Send any leftover funds to a new pool
        let change = PaymentPool {
            members: new_members,
            sequence: self.sequence + 1,
            sig_needed: self.sig_needed,
        };
        let mut tmpl = ctx.template().add_output(change.total(), &change, None)?;
        if all_payments.len() > 4 {
            // We'll use the contract from our last post to make the state
            // transitions more efficient!
            // Think about what else could be fun here though...
            tmpl = tmpl.add_output(
                spent,
                // TODO: Fix this treepay
                &TreePay {
                    participants: all_payments,
                    radix: 4,
                },
                None,
            )?;
        } else {
            for p in all_payments {
                tmpl = tmpl.add_output(
                    p.amount.try_into()?,
                    &Compiled::from_address(p.address, None),
                    None,
                )?;
            }
        }
        tmpl.into()
    }
}
REGISTER![PaymentPool, "logo.png"];
