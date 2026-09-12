//! An executable, non-CTV covenant emulation example.
//!
//! The fixed recipient and minimum are part of the program instance. A
//! continuation supplies candidate transactions; its witness only selects an
//! output to check. The oracle is trusted to evaluate this predicate honestly.

use bitcoin::bip32::{Xpriv, Xpub};
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::{Secp256k1, SecretKey};
use bitcoin::{Address, Amount, Network, OutPoint, Transaction, TxIn, TxOut};
use emulator_connect::program::{
    prepare_program_request, ProgramSigningRequest, ProgramSpendPath, WasmEvaluator,
};
use emulator_connect::CTVAvailable;
use sapio::contract::abi::studio::SapioStudioFormat;
use sapio::contract::*;
use sapio::*;
use sapio_base::effects::EffectPath;
use sapio_base::program::{EmulatedProgram, EvaluatorId, ProgramInstance};
use sapio_base::txindex::{TxIndex, TxIndexLogger};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::error::Error;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;

/// Fixed semantics selector interpreted by the registered example evaluator.
pub const PAY_AT_LEAST: &[u8] = b"pay-at-least/v1";
/// Sample funding amount, in satoshis.
pub const FUNDING_SATS: u64 = 20_000;
const FEE_SATS: u64 = 500;

/// Checked-in interpreter compiled reproducibly from `evaluators/pay-at-least`.
pub const PAY_AT_LEAST_WASM: &[u8] = include_bytes!("../../evaluators/artifacts/pay_at_least.wasm");

/// Identity committing the complete bounded WASM interpreter.
pub fn evaluator_id() -> EvaluatorId {
    EvaluatorId::for_wasm(PAY_AT_LEAST_WASM)
}

/// Register exact code instead of an ambient native evaluator callback.
pub fn pay_at_least_evaluator() -> WasmEvaluator {
    WasmEvaluator::new(PAY_AT_LEAST_WASM.to_vec()).expect("valid compiled payment interpreter")
}

/// Fully specified predicate parameters with an unambiguous binary encoding.
pub fn parameters(minimum: u64, recipient: &bitcoin::Script) -> Vec<u8> {
    let mut encoded = minimum.to_le_bytes().to_vec();
    encoded.extend_from_slice(&(recipient.len() as u32).to_le_bytes());
    encoded.extend_from_slice(recipient.as_bytes());
    encoded
}

/// The public instance needed to reproduce both evaluation and key derivation.
pub fn instance(minimum: u64, recipient: &bitcoin::Script) -> ProgramInstance {
    ProgramInstance::new(
        evaluator_id(),
        PAY_AT_LEAST.to_vec(),
        parameters(minimum, recipient),
    )
    .expect("bounded example program")
}

/// Prepare the compiled payment's key-path signature with an output-index witness.
pub fn signing_request(
    compiled: &Compiled,
    psbt: Psbt,
    output_index: u32,
) -> Result<ProgramSigningRequest, Box<dyn Error>> {
    let mut requirements = compiled
        .program_requirements()?
        .into_iter()
        .filter(|requirement| requirement.path == ProgramSpendPath::KeyPath);
    let requirement = requirements
        .next()
        .ok_or("payment has no program key path")?;
    if requirements.next().is_some() {
        return Err("payment has more than one program for its key path".into());
    }
    Ok(prepare_program_request(
        compiled,
        &requirement,
        psbt,
        0,
        output_index.to_le_bytes().to_vec(),
    )?)
}

/// Deterministic, disposable keys for this research example only.
pub fn example_root() -> Xpriv {
    // Testnet is the canonical BIP32 serialization network shared by regtest.
    Xpriv::new_master(Network::Testnet, &[91; 32]).unwrap()
}

/// A standard destination used in the example.
pub fn recipient(seed: u8) -> Address {
    let secp = Secp256k1::new();
    let key = bitcoin::CompressedPublicKey(bitcoin::secp256k1::PublicKey::from_secret_key(
        &secp,
        &SecretKey::from_slice(&[seed; 32]).unwrap(),
    ));
    Address::p2wpkh(&key, Network::Regtest)
}

/// Values supplied by a continuation, with no authority to change the guard.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PaymentCandidate {
    /// Amount paid to the contract's fixed recipient.
    pub amount: u64,
    /// Choose output order without changing the program identity.
    pub recipient_first: bool,
}

/// A continuation whose spending predicate is fixed before any effects run.
pub struct PaymentContract {
    emulation: EmulatedProgram,
    minimum: u64,
    recipient: Address,
}

impl PaymentContract {
    /// Build source data and its guard together so they cannot disagree.
    pub fn new(minimum: u64, recipient: Address, root: Xpub) -> Self {
        Self {
            emulation: EmulatedProgram::new(instance(minimum, &recipient.script_pubkey()), root)
                .expect("valid example root"),
            minimum,
            recipient,
        }
    }

    #[guard(policy, cached)]
    fn payment_policy(self) -> EmulatedProgram {
        self.emulation.clone()
    }

    #[continuation(guarded_by = "[Self::payment_policy]", coerce_args = "Ok", web_api)]
    fn pay(self, ctx: Context, candidate: Option<PaymentCandidate>) {
        let candidate = candidate.unwrap_or(PaymentCandidate {
            amount: self.minimum,
            recipient_first: true,
        });
        let change = FUNDING_SATS
            .checked_sub(FEE_SATS)
            .and_then(|available| available.checked_sub(candidate.amount))
            .ok_or_else(|| CompilationError::Custom("candidate exceeds example funding".into()))?;
        let paid = Compiled::from_address(self.recipient.clone(), Amount::ZERO);
        let change_address = Compiled::from_address(recipient(94), Amount::ZERO);
        let outputs = if candidate.recipient_first {
            [(candidate.amount, &paid), (change, &change_address)]
        } else {
            [(change, &change_address), (candidate.amount, &paid)]
        };
        let mut template = ctx.template();
        for (amount, output) in outputs {
            template = template.add_output(Amount::from_sat(amount), output, None)?;
        }
        template.add_fees(Amount::from_sat(FEE_SATS))?.into()
    }

    /// Compile using only source, public roots, and optional candidate effects.
    pub fn compile_candidates(
        &self,
        candidates: &[PaymentCandidate],
    ) -> Result<Compiled, CompilationError> {
        let effects: BTreeMap<_, _> = candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| (format!("candidate_{index}"), candidate))
            .collect();
        let effects = serde_json::from_value(serde_json::json!({
            "effects": {"payment/@action/pay/@suggested": effects}
        }))
        .expect("valid candidate effects");
        self.compile(Context::new(
            Network::Regtest,
            Amount::from_sat(FUNDING_SATS),
            sapio_base::LoweringPlan::Native,
            EffectPath::try_from("payment").unwrap(),
            Arc::new(effects),
            None,
        ))
    }
}

impl Contract for PaymentContract {
    declare! {updatable<Option<PaymentCandidate>>, Self::pay}
}

/// Attach candidates to a known synthetic funding transaction, without signing.
pub fn bind_candidates(compiled: &Compiled) -> Result<Vec<Psbt>, Box<dyn Error>> {
    let txindex: Rc<dyn TxIndex> = Rc::new(TxIndexLogger::new());
    let funding = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::absolute::LockTime::from_consensus(0),
        input: vec![TxIn::default()],
        output: vec![TxOut {
            value: bitcoin::Amount::from_sat(FUNDING_SATS),
            script_pubkey: (&compiled.address).into(),
        }],
    };
    let outpoint = OutPoint::new(txindex.add_tx(Arc::new(funding))?, 0);
    let bound = compiled.bind_psbt(outpoint, BTreeMap::new(), txindex, &CTVAvailable)?;
    let root = &bound.program[&compiled.root_path];
    root.txs
        .iter()
        .map(|tx| {
            let SapioStudioFormat::LinkedPSBT { psbt, .. } = tx;
            Psbt::from_str(psbt).map_err(Into::into)
        })
        .collect()
}
