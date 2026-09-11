//! Disposable fixtures and signing for the public template authorization contract.

use crate::program_example::{recipient, FUNDING_SATS};
use bitcoin::bip32::Xpub;
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::{Keypair, Message, Secp256k1, SecretKey};
use bitcoin::{Amount, Network, XOnlyPublicKey};
use emulator_connect::program::{ProgramSigningRequest, ProgramSpendPath, PSBT};
use sapio::contract::*;
use sapio::*;
use sapio_base::effects::EffectPath;
use sapio_base::fragments::{template_hash, KnownTweakProof};
use sapio_contrib::contracts::template_authorization::{Authorization, FragmentContract};
use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;

/// Disposable participant key used by the physical-internal-key example.
pub fn participant_key() -> Keypair {
    Keypair::from_secret_key(
        &Secp256k1::new(),
        &SecretKey::from_slice(&[96; 32]).unwrap(),
    )
}

/// Choose disposable destinations and a fee for the runnable example.
pub fn example_contract(authorization: Authorization, oracle_root: Xpub) -> FragmentContract {
    FragmentContract::new(
        authorization,
        oracle_root,
        recipient(92),
        recipient(94),
        Amount::from_sat(500),
    )
    .expect("valid disposable example root")
}

/// Compile the example's 6,000-satoshi candidate and any additional proposals.
pub fn compile_candidates(
    contract: &FragmentContract,
    amounts: &[u64],
) -> Result<Compiled, CompilationError> {
    let effects: BTreeMap<_, _> = std::iter::once(6_000)
        .chain(amounts.iter().copied())
        .enumerate()
        .map(|(index, amount)| (format!("candidate_{index}"), amount))
        .collect();
    let effects = serde_json::from_value(serde_json::json!({
        "effects": {"fragments/@action/pay/@suggested": effects}
    }))
    .expect("well-formed example effects");
    contract.compile(Context::new(
        Network::Regtest,
        Amount::from_sat(FUNDING_SATS),
        sapio_base::LoweringPlan::Native,
        EffectPath::try_from("fragments").unwrap(),
        Arc::new(effects),
        None,
    ))
}

/// Authorize a proposed transaction with an untweaked key and explicit annex.
pub fn signing_request(
    contract: &FragmentContract,
    mut psbt: Psbt,
    untweaked_authorizer: &Keypair,
    annex: Option<Vec<u8>>,
) -> Result<ProgramSigningRequest, Box<dyn Error>> {
    let input = psbt.inputs.first_mut().ok_or("candidate has no input")?;
    sapio_psbt::annex::set(input, annex)?;
    let hash = template_hash(&psbt.unsigned_tx, 0, sapio_psbt::annex::get(input)?)?;
    let signature = Secp256k1::new().sign_schnorr_no_aux_rand(
        &Message::from_digest_slice(hash.as_ref())?,
        untweaked_authorizer,
    );
    let (path, witness) = match contract.authorization() {
        Authorization::InternalKey(_) => (
            ProgramSpendPath::script_for(contract.program(), input)?,
            signature.as_ref().to_vec(),
        ),
        Authorization::KnownTweak => {
            let proof = KnownTweakProof::for_program(contract.program(), input.tap_merkle_root)?;
            let prevout = input.witness_utxo.as_ref().ok_or("missing spent output")?;
            if !prevout.script_pubkey.is_p2tr() {
                return Err("spent output is not Taproot".into());
            }
            proof.check_output(XOnlyPublicKey::from_slice(
                &prevout.script_pubkey.as_bytes()[2..],
            )?)?;
            // Key-path proof needs the output and Merkle root, not a claimed
            // internal key or a control block from an unrelated script path.
            input.tap_scripts.clear();
            input.tap_key_origins.clear();
            input.tap_internal_key = None;
            (ProgramSpendPath::KeyPath, proof.witness(&signature))
        }
    };
    Ok(ProgramSigningRequest {
        instance: contract.program().instance().clone(),
        input_index: 0,
        witness,
        path,
        psbt: PSBT(psbt),
    })
}
