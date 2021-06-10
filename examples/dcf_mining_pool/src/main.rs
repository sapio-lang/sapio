// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
use crate::contract::Context;
use crate::miner_payout::MiningPayout;
use crate::miner_payout::PoolShare;
use bitcoin::hash_types::BlockHash;
use bitcoin::Amount;
use bitcoin::Block;
use bitcoin::PublicKey;
use bitcoin::Script;
use bitcoincore_rpc_async as rpc;
use rpc::RpcApi;
use sapio::contract::Compilable;
use sapio::contract::Contract;
use sapio::*;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::RwLock;

mod miner_payout;
struct BlockNotes {
    block: Block,
    key: Option<PublicKey>,
    participated: Option<bool>,
    reward: Amount,
}
impl BlockNotes {
    fn from_block(block: Block) -> BlockNotes {
        let mut key = None;
        // Extract key from first OP_RETURN 123 45 67 89 <33 bytes>
        if let Some(coinbase) = block.coinbase() {
            for out in coinbase.output.iter() {
                if out.script_pubkey.is_op_return() {
                    match out.script_pubkey.as_bytes() {
                        [106, 123, 45, 67, 89, tail @ ..] => {
                            if tail.len() == 33 {
                                key = PublicKey::from_slice(tail).ok();
                                if key.is_some() {
                                    break;
                                }
                            }
                        }
                        _ => continue,
                    }
                }
            }
        }
        let participated = if key.is_none() { Some(false) } else { None };
        BlockNotes {
            block,
            key,
            participated,
            reward: Amount::from_sat(0),
        }
    }
}
struct Coordinator {
    cache: HashMap<BlockHash, Arc<RwLock<BlockNotes>>>,
    client: rpc::Client,
    ctx: Context,
}

impl Coordinator {
    async fn compute_for_block(
        &mut self,
        tip_in: &BlockHash,
        n: usize,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut tip = *tip_in;
        let mut to_scan = vec![];
        // ensure our cache has all relevant info
        let cache = &mut self.cache;
        for i in 0..n {
            let locked_note = match cache.entry(tip) {
                Entry::Occupied(o) => o.into_mut(),
                Entry::Vacant(v) => {
                    let block = self.client.get_block(&tip).await?;
                    let mut note = BlockNotes::from_block(block);
                    if note.key.is_some() {
                        let stats: serde_json::Value = self
                            .client
                            .call("getblockstats", &[serde_json::to_value(tip)?])
                            .await?;
                        let subsidy = stats
                            .get("subsidy")
                            .and_then(serde_json::Value::as_u64)
                            .ok_or("blockstates error")?;
                        let fee = stats
                            .get("totalfee")
                            .and_then(serde_json::Value::as_u64)
                            .ok_or("blockstats error")?;
                        note.reward = Amount::from_sat(fee + subsidy);
                    }
                    v.insert(Arc::new(RwLock::new(note)))
                }
            };
            let note = locked_note.read().unwrap();
            // fast response of the block has no key definitely no participating
            if i == 0 && note.participated == Some(false) {
                return Ok(false);
            }
            // get stats for these blocks...
            if note.key.is_some() && note.participated.is_none() {
                // scan all contendors except the first tip
                if i > 0 {
                    to_scan.push(locked_note.clone());
                }
            }
            tip = note.block.header.prev_blockhash;
        }
        // scan all the parents we aren't sure about
        // goes from oldest to newest
        while let Some(scan) = to_scan.pop() {
            let h = scan.read().unwrap().block.block_hash();
            let v: Pin<Box<dyn Future<Output = _>>> = Box::pin(self.compute_for_block(&h, n));
            v.await;
        }

        let tip = *tip_in;
        let mut known_participants = vec![];
        // ensure our cache has all relevant info
        for _ in 0..n {
            let note = self.cache[&tip].clone();
            let note_r = note.read().unwrap();
            if note_r.participated == Some(true) {
                std::mem::drop(note_r);
                known_participants.push(note);
            }
        }
        let mp = MiningPool {
            blocks: known_participants,
            tip: self.cache[tip_in].clone(),
        };
        let output = mp.compile(&self.ctx.with_amount(mp.tip.read().unwrap().reward)?)?;
        let script: Script = output.address.into();

        let mut result = false;
        if let Some(coinbase) = mp.tip.read().unwrap().block.coinbase() {
            for out in coinbase.output.iter() {
                if out.script_pubkey == script && out.value == output.amount_range.max().as_sat() {
                    result = true;
                    break;
                }
            }
        }
        let mut tip = mp.tip.write().unwrap();
        tip.participated = Some(result);
        return Ok(result);
    }
}

use std::sync::Arc;
struct MiningPool {
    blocks: Vec<Arc<RwLock<BlockNotes>>>,
    tip: Arc<RwLock<BlockNotes>>,
}

use sapio::contract::error::CompilationError;
impl MiningPool {
    then! {
        fn pay_miners(self, ctx) {
            let mut blocks = self.blocks.clone();
            blocks.push(self.tip.clone());
            blocks.sort_by_cached_key(|a| {
                a.read().unwrap().block
                .header
                .block_hash()
            });
            let participants : Vec<PoolShare> = blocks.iter().map(|note|
                Ok(PoolShare {
                    amount: Amount::from_sat(0),
                    // guaranteed if here to have a pk
                    key: note.read().unwrap().key.unwrap()
                })
            ).collect::<Result<_,CompilationError>>()?;
            let mut ctx_extra_funding :Context= ctx.clone();
            ctx_extra_funding.add_amount(Amount::from_btc(21_000_000.0).unwrap());

            let mut contract = MiningPayout {
                    /// all of the payments needing to be sent
                    participants,
                    radix: 4,
                    fee_sats_per_tx: Amount::from_sat(100),
                };
            let fee_estimate =
                contract.compile(ctx)?.amount_range.max();
            let reward = self.tip.read().unwrap().reward - fee_estimate;
            let reward_per_miner = ( reward) / (blocks.len() as u64);

            for reward in contract.participants.iter_mut() {
                reward.amount = reward_per_miner;
            }
            ctx.template().add_output(
                reward,
                &contract,
                None
            )?.into()
        }
    }
}
impl Contract for MiningPool {
    declare! {then, Self::pay_miners}
    declare! {non updatable}
}

fn main() {
    loop {}
}
