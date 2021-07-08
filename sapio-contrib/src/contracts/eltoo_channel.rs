// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! An example of how one might begin building a payment channel contract in Sapio using
//! Eltoo
use bitcoin::util::bip32::ChildNumber;
use bitcoin::util::bip32::ExtendedPubKey;
use contract::actions::*;
use contract::*;
use sapio::contract::actions::ConditionalCompileType;
use sapio::contract::error::CompilationError;
use sapio::template::Output;
use sapio::*;
use sapio_base::timelocks::RelHeight;
use sapio_base::Clause;

use bitcoin;
use bitcoin::secp256k1::*;
use bitcoin::util::amount::{Amount, CoinAmount};
use rand::rngs::OsRng;
use sapio_base::timelocks::{AbsTime, AnyAbsTimeLock};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;

use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use sapio::template::Builder as Template;
use std::convert::TryFrom;
use std::convert::TryInto;

/// Args are some messages that can be passed to a Channel instance
#[derive(Clone)]
pub struct Update {
    /// the balances of the channel
    resolution: Vec<Output>,
    /// the channel seq
    sequence: AbsTime,
}

#[derive(Clone)]
struct OpenChannel {
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    alice_u: bitcoin::PublicKey,
    bob_u: bitcoin::PublicKey,
    pending_update: Option<Update>,
}
impl OpenChannel {
    guard! {fn signed_update(self, ctx) {Clause::And(vec![Clause::Key(self.alice_u), Clause::Key(self.bob_u)])}}
    guard! {
        fn newer_sequence_check(self, ctx) {
            self.pending_update.as_ref().map(|u| {
                u.sequence.get()+1
            }).map(AbsTime::try_from).transpose().expect("Shouldn't fail").map(Clause::from).unwrap_or(Clause::Trivial)
        }
    }
    finish! {
        guarded_by: [Self::signed_update, Self::newer_sequence_check]
        fn update_state(self, ctx, o) {
            if let Some(update) = o {
                if update.sequence > (1_000_000_000u32).try_into()? {
                    Err(CompilationError::TerminateCompilation)?;
                }
                let prior_seq =
                    self.pending_update.as_ref().map(|u|u.sequence).unwrap_or(1u32.try_into()?);
                if update.sequence <= prior_seq {
                    Err(CompilationError::TerminateCompilation)?;
                }
                ctx.template().add_output(
                    ctx.funds(),
                    &OpenChannel {
                        pending_update: Some(update.clone()), ..self.clone()
                    }
                    ,
                    None
                )?.set_lock_time(AnyAbsTimeLock::from(update.sequence.clone()))?.into()

            } else {
                Ok(Box::new(std::iter::empty()))
            }
        }
    }

    compile_if! {
        fn triggered(self, ctx) {
            if self.pending_update.is_some() {
                ConditionalCompileType::NoConstraint
            } else {
                ConditionalCompileType::Never
            }
        }
    }

    guard! {fn timeout(self, ctx) { Clause::Older(100) }}
    then! {
        compile_if: [Self::triggered]
        guarded_by: [Self::timeout]
        fn complete_update(self, ctx) {
            let mut template = ctx.template().set_sequence(-1, RelHeight::from(100).into())?;
            for out in self.pending_update.as_ref().expect("it is triggered").resolution.iter() {
                template = template.add_output(out.amount, &out.contract, Some(out.metadata.clone()))?;
            }
            template.into()
        }
    }

    guard! {
        fn sign_cooperative_close(self, ctx) {
            Clause::And(vec![
                    Clause::Key(self.alice),
                    Clause::Key(self.bob) ])
        }
    }

    compile_if! {
        fn untriggered(self, ctx) {
            if self.pending_update.is_some() {
                ConditionalCompileType::Never
            } else {
                ConditionalCompileType::NoConstraint
            }
        }
    }
    finish! {
        compile_if: [Self::untriggered]
        guarded_by: [Self::sign_cooperative_close]
        fn coop_close(self, ctx, o) {
            Ok(Box::new(std::iter::empty()))
        }
    }
}

impl Contract for OpenChannel {
    declare! {updatable<Update>, Self::update_state,  Self::coop_close}
    declare! {then, Self::complete_update}
}
