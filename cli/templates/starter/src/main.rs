//! Export a synthetic demonstration; this runner never contacts a wallet or node.

mod contract;
#[cfg(test)]
mod tests;

use bitcoin::bip32::{Xpriv, Xpub};
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{Address, Amount, Network, OutPoint, Transaction, TxIn, TxOut};
use clap::{Parser, Subcommand};
use contract::{Payment, PaymentRequest};
use emulator_connect::program::{
    ProgramCapability, ProgramEvidence, ProgramSpendPath, SpendAssets,
};
use sapio::contract::{Compilable, Compiled, Context};
use sapio_base::effects::EffectPath;
use sapio_base::program::EvaluatorId;
use sapio_base::LoweringPlan;
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const EVALUATOR_WASM: &[u8] = include_bytes!("../pay_at_least.wasm");
const FUNDING_SATS: u64 = 20_000;
const MINIMUM_SATS: u64 = 5_000;
const EVIDENCE_CODEC: &str = "pay-at-least/output-index-v1";

#[derive(Parser)]
#[command(about = "Compile a payment contract and export synthetic demo inputs")]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new directory containing the artifact and explicit signing inputs.
    Build {
        directory: PathBuf,
        /// Proposed recipient payment in satoshis; the evaluator requires 5000.
        #[arg(long, default_value_t = 6_000)]
        amount: u64,
    },
}

struct Demo {
    artifact: Compiled,
    psbt: Psbt,
    assets: SpendAssets,
    evidence: Vec<ProgramEvidence>,
    oracle: Xpriv,
}

fn disposable_root(seed: u8) -> Result<Xpriv, bitcoin::bip32::Error> {
    // Published deterministic keys make this example reproducible, never private.
    Xpriv::new_master(Network::Regtest, &[seed; 32])
}

fn destination(seed: u8) -> Result<Address, bitcoin::bip32::Error> {
    let secp = Secp256k1::new();
    let key = Xpub::from_priv(&secp, &disposable_root(seed)?).public_key;
    Ok(Address::p2tr(
        &secp,
        key.x_only_public_key().0,
        None,
        Network::Regtest,
    ))
}

fn demo(amount: u64) -> Result<Demo, Box<dyn Error>> {
    let oracle = disposable_root(91)?;
    let contract = Payment::new(
        destination(92)?,
        destination(94)?,
        Amount::from_sat(MINIMUM_SATS),
        EvaluatorId::for_wasm(EVALUATOR_WASM),
        Xpub::from_priv(&Secp256k1::new(), &oracle),
    )?;
    let root = EffectPath::try_from("payment")?;
    let effects = Payment::pay_action().request(
        &root,
        &PaymentRequest {
            amount_sats: amount,
        },
    )?;
    let artifact = contract.compile(Context::new(
        Network::Regtest,
        Amount::from_sat(FUNDING_SATS),
        // The policy is explicitly a Program. This example emits no native CTV.
        LoweringPlan::Native,
        root,
        Arc::new(effects),
        None,
    ))?;
    artifact.validate()?;
    let template = artifact
        .suggested_txs
        .values()
        .next()
        .ok_or("missing payment proposal")?;
    let funding = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: vec![TxIn::default()],
        output: vec![TxOut {
            value: Amount::from_sat(FUNDING_SATS),
            script_pubkey: (&artifact.address).into(),
        }],
    };
    let mut transaction = template.tx.clone();
    transaction.input[0].previous_output = OutPoint::new(funding.compute_txid(), 0);
    let mut psbt = Psbt::from_unsigned_tx(transaction)?;
    psbt.inputs[0].witness_utxo = Some(funding.output[0].clone());
    psbt.inputs[0].non_witness_utxo = Some(funding);
    let mut requirements = artifact
        .program_requirements()?
        .into_iter()
        .filter(|requirement| requirement.path == ProgramSpendPath::KeyPath);
    let requirement = requirements.next().ok_or("missing program requirement")?;
    if requirements.next().is_some() || requirement.path != ProgramSpendPath::KeyPath {
        return Err("expected a program key-path spend".into());
    }
    let assets = SpendAssets {
        programs: vec![ProgramCapability {
            requirement: requirement.clone(),
            codec: EVIDENCE_CODEC.into(),
            evidence_available: true,
            signer_available: true,
        }],
        ..Default::default()
    };
    let evidence = vec![ProgramEvidence {
        requirement: requirement.clone(),
        codec: EVIDENCE_CODEC.into(),
        // The recipient is output zero. Evidence selects the output to inspect.
        witness: 0_u32.to_le_bytes().to_vec(),
    }];
    Ok(Demo {
        artifact,
        psbt,
        assets,
        evidence,
        oracle,
    })
}

fn build(directory: &Path, amount: u64) -> Result<(), Box<dyn Error>> {
    let demo = demo(amount)?;
    let files = [
        ("artifact.json", serde_json::to_vec_pretty(&demo.artifact)?),
        (
            "funded.psbt",
            base64::encode(demo.psbt.serialize()).into_bytes(),
        ),
        ("assets.json", serde_json::to_vec_pretty(&demo.assets)?),
        ("evidence.json", serde_json::to_vec_pretty(&demo.evidence)?),
        ("oracle.key", demo.oracle.encode().to_vec()),
        ("pay_at_least.wasm", EVALUATOR_WASM.to_vec()),
    ];
    fs::create_dir(directory)?;
    for (name, bytes) in files {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(name))?
            .write_all(&bytes)?;
    }
    println!(
        "Created {} with a {amount}-sat proposal (minimum {MINIMUM_SATS}).",
        directory.display()
    );
    println!("Funding is synthetic and oracle.key is public demonstration material. Nothing was signed or broadcast.");
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    match Arguments::parse().command {
        Command::Build { directory, amount } => build(&directory, amount),
    }
}
