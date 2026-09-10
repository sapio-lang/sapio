// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Compile a mining reward payout offline. No node, server, or broadcast is used.
use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
use bitcoin::{Amount, Network, XOnlyPublicKey};
use miner_payout::MiningPayout;
use sapio::contract::{Compilable, Context};
use sapio_base::covenant::LoweringPlan;
use sapio_base::effects::EffectPath;
use serde::Deserialize;
use std::io::Read;
use std::sync::Arc;

mod miner_payout;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    miners: Vec<XOnlyPublicKey>,
    reward_sats: u64,
    radix: usize,
    fee_sats_per_tx: u64,
}

fn demo() -> Request {
    Request {
        miners: (1..=5)
            .map(|byte| {
                Keypair::from_secret_key(
                    &Secp256k1::new(),
                    &SecretKey::from_slice(&[byte; 32]).unwrap(),
                )
                .x_only_public_key()
                .0
            })
            .collect(),
        reward_sats: 50_003,
        radix: 4,
        fee_sats_per_tx: 100,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let request = match args.next().as_deref() {
        None => demo(),
        Some("--help" | "-h") => {
            println!("dcf_mining_pool [INPUT.json|-]\nNo arguments compile a deterministic demonstration. Use - to read JSON from stdin.");
            return Ok(());
        }
        Some(path) => {
            let mut input = String::new();
            if path == "-" {
                std::io::stdin().read_to_string(&mut input)?;
            } else {
                input = std::fs::read_to_string(path)?;
            }
            serde_json::from_str(&input)?
        }
    };
    if args.next().is_some() {
        return Err("Expected at most one JSON input path".into());
    }
    let payout = MiningPayout::from_reward(
        request.miners,
        Amount::from_sat(request.reward_sats),
        request.radix,
        Amount::from_sat(request.fee_sats_per_tx),
    )?;
    let compiled = payout.compile(Context::new(
        Network::Regtest,
        payout.funding_required()?,
        LoweringPlan::Native,
        EffectPath::try_from("mining_payout")?,
        Arc::new(Default::default()),
        None,
    ))?;
    compiled.validate()?;
    serde_json::to_writer_pretty(
        std::io::stdout().lock(),
        &serde_json::json!({
            "network": "regtest", "enforcement": "native_ctv_research",
            "funding_satoshis": request.reward_sats,
            "payouts": payout.participants,
            "contract": compiled,
        }),
    )?;
    Ok(())
}
