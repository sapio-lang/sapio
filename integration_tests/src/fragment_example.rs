//! Disposable fixtures and signing for the public template authorization contract.

use crate::program_example::{recipient, FUNDING_SATS};
use bitcoin::bip32::Xpub;
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::{Keypair, Message, Secp256k1, SecretKey};
use bitcoin::{Amount, Network, XOnlyPublicKey};
use emulator_connect::program::{prepare_program_request, ProgramSigningRequest, ProgramSpendPath};
use sapio::contract::abi::object::ProgramRequirement;
use sapio::contract::*;
use sapio::*;
use sapio_base::effects::EffectPath;
use sapio_base::fragments::{
    template_authorization_wasm_instance, template_hash, KnownTweakProof, TemplateKey,
};
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

/// Select the exact program and spend path declared by the compiled contract.
pub fn authorization_requirement(
    compiled: &Compiled,
    authorization: Authorization,
) -> Result<ProgramRequirement, Box<dyn Error>> {
    let key = match authorization {
        Authorization::InternalKey(_) => TemplateKey::InternalKey,
        Authorization::KnownTweak => TemplateKey::KnownTweak,
    };
    let instance = template_authorization_wasm_instance(key);
    let requirements = compiled.program_requirements()?;
    let mut matching = requirements.into_iter().filter(|requirement| {
        requirement.program.instance() == &instance
            && matches!(
                (authorization, requirement.path),
                (
                    Authorization::InternalKey(_),
                    ProgramSpendPath::ScriptPath(_)
                ) | (Authorization::KnownTweak, ProgramSpendPath::KeyPath)
            )
    });
    let requirement = matching
        .next()
        .ok_or("artifact has no matching authorization path")?;
    if matching.next().is_some() {
        return Err("artifact has ambiguous authorization paths".into());
    }
    Ok(requirement)
}

/// Authorize a proposed transaction using only its artifact and an explicit mode.
pub fn signing_request(
    compiled: &Compiled,
    authorization: Authorization,
    mut psbt: Psbt,
    untweaked_authorizer: &Keypair,
    annex: Option<Vec<u8>>,
) -> Result<ProgramSigningRequest, Box<dyn Error>> {
    let requirement = authorization_requirement(compiled, authorization)?;
    let input = psbt.inputs.first_mut().ok_or("candidate has no input")?;
    sapio_psbt::annex::set(input, annex)?;
    let hash = template_hash(&psbt.unsigned_tx, 0, sapio_psbt::annex::get(input)?)?;
    let signature = Secp256k1::new().sign_schnorr_no_aux_rand(
        &Message::from_digest_slice(hash.as_ref())?,
        untweaked_authorizer,
    );
    let witness = match authorization {
        Authorization::InternalKey(key) => {
            if untweaked_authorizer.x_only_public_key().0 != key {
                return Err("authorization key differs from the selected internal key".into());
            }
            signature.as_ref().to_vec()
        }
        Authorization::KnownTweak => {
            let mut proof_input = bitcoin::psbt::Input::default();
            compiled
                .descriptor
                .as_ref()
                .ok_or("artifact has no descriptor")?
                .update_psbt_input(&mut proof_input)?;
            let proof =
                KnownTweakProof::for_program(&requirement.program, proof_input.tap_merkle_root)?;
            let prevout = input.witness_utxo.as_ref().ok_or("missing spent output")?;
            if !prevout.script_pubkey.is_p2tr() {
                return Err("spent output is not Taproot".into());
            }
            proof.check_output(XOnlyPublicKey::from_slice(
                &prevout.script_pubkey.as_bytes()[2..],
            )?)?;
            proof.witness(&signature)
        }
    };
    let mut request = prepare_program_request(compiled, &requirement, psbt, 0, witness)?;
    if matches!(authorization, Authorization::KnownTweak) {
        // The opening authenticates the output key without revealing an
        // internal key or a control block from a different spending path.
        let input = &mut request.psbt.0.inputs[0];
        input.tap_scripts.clear();
        input.tap_key_origins.clear();
        input.tap_internal_key = None;
    }
    Ok(request)
}
