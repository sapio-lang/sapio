//! Compile public eltoo contracts and bind authorized templates to real coins.
//!
//! The caller supplies authenticated funding and signing keys. Regtest network,
//! continuation effects, fee sponsorship and PSBT assembly belong to this runner.

use bitcoin::key::TapTweak;
use bitcoin::psbt::{Input, Psbt};
use bitcoin::secp256k1::{schnorr::Signature, Keypair, Message, Secp256k1};
use bitcoin::sighash::{Prevouts, SighashCache};
use bitcoin::taproot::TapLeafHash;
use bitcoin::{
    taproot, Address, Amount, Network, OutPoint, ScriptBuf, TapSighashType, Transaction, TxOut,
    XOnlyPublicKey,
};
use emulator_connect::program::spend_plan::{ProgramCapability, ProgramEvidence, SpendPath};
use emulator_connect::program::{
    prepare_spend, ProgramSigningRequest, ProgramSpendPath, SpendAssets,
};
use sapio::contract::abi::object::ProgramRequirement;
use sapio::contract::{Compilable, Compiled, Context};
use sapio::template::Template;
use sapio_base::effects::MapEffectDB;
use sapio_base::fragments::{
    template_authorization_wasm_instance, template_hash, templatehash_wasm_instance, TemplateKey,
};
use sapio_base::program::{EmulatedProgram, ProgramInstance};
use sapio_contrib::contracts::eltoo::{Channel, State, Terms};
use std::sync::Arc;

/// Errors in the example's compilation, binding and signing helpers.
pub type Error = Box<dyn std::error::Error>;

fn context(terms: &Terms) -> Result<Context, Error> {
    Ok(Context::new(
        Network::Regtest,
        Amount::from_sat(terms.capacity()),
        sapio_base::LoweringPlan::Native,
        "eltoo".try_into()?,
        Arc::new(Default::default()),
        None,
    ))
}

fn compile_with(source: &Channel, effects: MapEffectDB) -> Result<Compiled, Error> {
    Ok(source.compile(Context::new(
        Network::Regtest,
        Amount::from_sat(source.terms().capacity()),
        sapio_base::LoweringPlan::Native,
        "eltoo".try_into()?,
        Arc::new(effects),
        None,
    ))?)
}

/// Compile the output's fixed policy without adding transition candidates.
pub fn compile(source: &Channel) -> Result<Compiled, Error> {
    compile_with(source, MapEffectDB::default())
}

/// Compile a newer state as a candidate under the source's unchanged policy.
pub fn compile_update(source: &Channel, target: State) -> Result<Compiled, Error> {
    source.terms().validate(target)?;
    if source
        .current_state()
        .is_some_and(|old| target.number <= old.number)
    {
        return Err("updates must advance the state number".into());
    }
    compile_with(
        source,
        Channel::update_action().request(&"eltoo".try_into()?, &target)?,
    )
}

/// Compile a state's delayed settlement candidate.
pub fn compile_settlement(source: &Channel) -> Result<Compiled, Error> {
    if source.current_state().is_none() {
        return Err("funding has no settlement path".into());
    }
    compile_with(
        source,
        Channel::settle_action().request(&"eltoo".try_into()?, &())?,
    )
}

/// Canonical update transaction, before attaching channel and sponsor outpoints.
pub fn update_transaction(terms: &Terms, target: State) -> Result<Transaction, Error> {
    Ok(update_template(terms, target)?.tx)
}

/// Canonical update and its named funding requirements, retained through signing.
pub fn update_template(terms: &Terms, target: State) -> Result<Template, Error> {
    Ok(only_candidate(&compile_update(&terms.funding(), target)?)?.clone())
}

/// Canonical settlement transaction, independent of its funding outpoints.
pub fn settlement_transaction(terms: &Terms, state: State) -> Result<Transaction, Error> {
    Ok(settlement_template(terms, state)?.tx)
}

/// Canonical settlement and its explicit local sponsor-fee ceiling.
pub fn settlement_template(terms: &Terms, state: State) -> Result<Template, Error> {
    Ok(terms.settlement_template(state, context(terms)?)?)
}

/// Verify witnesses, check the retained funding rules, and then extract a transaction.
pub fn finalize_candidate(template: &Template, psbt: Psbt) -> Result<Transaction, Error> {
    let finalized = sapio_psbt::finalize::finalize(psbt, &Secp256k1::new())
        .map_err(|(_, errors)| format!("eltoo finalization failed: {errors:?}"))?;
    template.check_funded_psbt(&finalized)?;
    Ok(finalized.extract_tx()?)
}

fn requirement_for(
    compiled: &Compiled,
    instance: ProgramInstance,
) -> Result<ProgramRequirement, Error> {
    let requirements = compiled.program_requirements()?;
    let mut matching = requirements.into_iter().filter(|requirement| {
        requirement.program.instance() == &instance
            && matches!(requirement.path, ProgramSpendPath::ScriptPath(_))
    });
    let requirement = matching
        .next()
        .ok_or("artifact has no matching eltoo program path")?;
    if matching.next().is_some() {
        return Err("artifact has ambiguous eltoo program paths".into());
    }
    Ok(requirement)
}

/// Select the artifact's exact internal-key template authorization program.
pub fn update_requirement(compiled: &Compiled) -> Result<ProgramRequirement, Error> {
    requirement_for(
        compiled,
        template_authorization_wasm_instance(TemplateKey::InternalKey),
    )
}

/// Select the artifact's exact predicate for this settlement transaction.
pub fn settlement_requirement(
    compiled: &Compiled,
    transaction: &Transaction,
) -> Result<ProgramRequirement, Error> {
    requirement_for(
        compiled,
        templatehash_wasm_instance(template_hash(transaction, 0, None)?),
    )
}

/// Read a state's settlement program from its compiled artifact.
pub fn settlement_program(source: &Channel) -> Result<EmulatedProgram, Error> {
    let compiled = compile_settlement(source)?;
    Ok(settlement_requirement(&compiled, &only_candidate(&compiled)?.tx)?.program)
}

fn spending_input(compiled: &Compiled) -> Result<Input, Error> {
    let descriptor = compiled
        .descriptor
        .as_ref()
        .filter(|descriptor| descriptor.script_pubkey().is_p2tr())
        .ok_or("expected a compiled Taproot descriptor")?;
    let mut input = Input::default();
    descriptor.update_psbt_input(&mut input)?;
    Ok(input)
}

fn leaf_for(compiled: &Compiled, requirement: &ProgramRequirement) -> Result<ScriptBuf, Error> {
    let input = spending_input(compiled)?;
    let ProgramSpendPath::ScriptPath(hash) = requirement.path else {
        return Err("expected a program script path".into());
    };
    input
        .tap_scripts
        .values()
        .find(|(script, version)| TapLeafHash::from_script(script, *version) == hash)
        .map(|(script, _)| script.clone())
        .ok_or_else(|| "program leaf absent from this channel output".into())
}

/// Find the update program's exact leaf in the compiled artifact.
pub fn update_leaf(source: &Channel) -> Result<ScriptBuf, Error> {
    let compiled = compile(source)?;
    leaf_for(&compiled, &update_requirement(&compiled)?)
}

/// Find the settlement program's exact leaf in the compiled artifact.
pub fn settlement_leaf(source: &Channel) -> Result<ScriptBuf, Error> {
    let compiled = compile_settlement(source)?;
    let requirement = settlement_requirement(&compiled, &only_candidate(&compiled)?.tx)?;
    leaf_for(&compiled, &requirement)
}

/// Attach a verified channel coin to the compiler's spending proof.
pub fn input(source: &Channel, coin: &Coin) -> Result<Input, Error> {
    input_from_artifact(&compile(source)?, coin)
}

/// Attach a channel coin using only its compiled artifact.
pub fn input_from_artifact(compiled: &Compiled, coin: &Coin) -> Result<Input, Error> {
    if coin.txout.value != compiled.required_input_amount
        || coin.txout.script_pubkey != ScriptBuf::from(&compiled.address)
    {
        return Err("channel funding does not match the compiled output".into());
    }
    let mut input = spending_input(compiled)?;
    input.witness_utxo = Some(coin.txout.clone());
    Ok(input)
}

fn only_candidate(compiled: &Compiled) -> Result<&Template, Error> {
    if compiled.suggested_txs.len() != 1 || !compiled.ctv_to_tx.is_empty() {
        return Err("expected exactly one continuation candidate and no CTV branches".into());
    }
    Ok(compiled.suggested_txs.values().next().unwrap())
}

/// A funding assertion; the caller must obtain actual prevouts from its node.
#[derive(Clone, Debug)]
pub struct Coin {
    pub outpoint: OutPoint,
    pub txout: TxOut,
}

/// Replaceable key-path sponsor; its entire value becomes this transaction's fee.
#[derive(Clone, Debug)]
pub struct Sponsor {
    pub coin: Coin,
    pub internal_key: XOnlyPublicKey,
}

/// Bind an independently authorized template to authenticated channel and sponsor coins.
pub fn attach_inputs(
    mut transaction: Transaction,
    channel: Coin,
    mut input: Input,
    sponsor: Sponsor,
) -> Result<Psbt, Error> {
    if transaction.version != bitcoin::transaction::Version::TWO
        || transaction.input.len() != 2
        || sponsor.coin.txout.value.to_sat() == 0
    {
        return Err("eltoo requires version two, two inputs and a positive sponsor".into());
    }
    let script = Address::p2tr(
        &Secp256k1::new(),
        sponsor.internal_key,
        None,
        Network::Regtest,
    )
    .script_pubkey();
    if sponsor.coin.txout.script_pubkey != script {
        return Err("sponsor coin does not match its untweaked P2TR key".into());
    }
    let output_sum = transaction
        .output
        .iter()
        .try_fold(0u64, |sum, output| sum.checked_add(output.value.to_sat()))
        .ok_or("output sum overflows")?;
    if channel.txout.value.to_sat() != output_sum
        || input.witness_utxo.as_ref() != Some(&channel.txout)
    {
        return Err("channel capacity and authenticated prevout must match all outputs".into());
    }
    transaction.input[0].previous_output = channel.outpoint;
    transaction.input[1].previous_output = sponsor.coin.outpoint;
    input.non_witness_utxo = None;
    let mut psbt = Psbt::from_unsigned_tx(transaction)?;
    psbt.inputs[0] = input;
    psbt.inputs[1] = Input {
        witness_utxo: Some(sponsor.coin.txout),
        tap_internal_key: Some(sponsor.internal_key),
        ..Input::default()
    };
    Ok(psbt)
}

/// Jointly authorize a target template once, independent of its funding outpoints.
pub fn authorize_update(terms: &Terms, target: State, joint: &Keypair) -> Result<Signature, Error> {
    if joint.x_only_public_key().0 != terms.joint_key() {
        return Err("authorization key does not match the channel's joint key".into());
    }
    let hash = template_hash(&update_transaction(terms, target)?, 0, None)?;
    Ok(Secp256k1::new()
        .sign_schnorr_no_aux_rand(&Message::from_digest_slice(hash.as_ref())?, joint))
}

/// Compile a newer candidate and prepare its artifact-declared authorization.
pub fn update_request(
    source: &Channel,
    target: State,
    channel: Coin,
    sponsor: Sponsor,
    authorization: &Signature,
) -> Result<ProgramSigningRequest, Error> {
    update_request_from_artifact(
        &compile_update(source, target)?,
        channel,
        sponsor,
        authorization,
    )
}

/// Bind and authorize an exported update candidate without its Rust contract.
pub fn update_request_from_artifact(
    compiled: &Compiled,
    channel: Coin,
    sponsor: Sponsor,
    authorization: &Signature,
) -> Result<ProgramSigningRequest, Error> {
    let requirement = update_requirement(compiled)?;
    let input = input_from_artifact(compiled, &channel)?;
    let template = only_candidate(compiled)?;
    let psbt = attach_inputs(template.tx.clone(), channel, input, sponsor)?;
    template.check_funded_psbt(&psbt)?;
    prepare_authorization(
        compiled,
        requirement,
        psbt,
        "eltoo/update-signature/v2",
        authorization.as_ref().to_vec(),
    )
}

/// Compile the delayed exit and prepare its artifact-declared predicate.
pub fn settlement_request(
    source: &Channel,
    channel: Coin,
    sponsor: Sponsor,
) -> Result<ProgramSigningRequest, Error> {
    settlement_request_from_artifact(&compile_settlement(source)?, channel, sponsor)
}

/// Bind an exported settlement candidate without reconstructing its contract.
pub fn settlement_request_from_artifact(
    compiled: &Compiled,
    channel: Coin,
    sponsor: Sponsor,
) -> Result<ProgramSigningRequest, Error> {
    let template = only_candidate(compiled)?;
    let requirement = settlement_requirement(compiled, &template.tx)?;
    let input = input_from_artifact(compiled, &channel)?;
    let psbt = attach_inputs(template.tx.clone(), channel, input, sponsor)?;
    template.check_funded_psbt(&psbt)?;
    prepare_authorization(
        compiled,
        requirement,
        psbt,
        "eltoo/settlement-empty/v2",
        vec![],
    )
}

fn prepare_authorization(
    compiled: &Compiled,
    requirement: ProgramRequirement,
    psbt: Psbt,
    codec: &str,
    witness: Vec<u8>,
) -> Result<ProgramSigningRequest, Error> {
    let ProgramSpendPath::ScriptPath(leaf) = requirement.path else {
        return Err("eltoo authorization must use a script path".into());
    };
    let assets = SpendAssets {
        programs: vec![ProgramCapability {
            requirement: requirement.clone(),
            codec: codec.into(),
            evidence_available: true,
            signer_available: false,
        }],
        ..Default::default()
    };
    let evidence = [ProgramEvidence {
        requirement,
        codec: codec.into(),
        witness,
    }];
    let mut prepared = prepare_spend(
        compiled,
        SpendPath::ScriptPath(leaf),
        psbt,
        0,
        &assets,
        &evidence,
    )?;
    if prepared.program_requests.len() != 1 {
        return Err("eltoo branch needs exactly one unsigned program".into());
    }
    Ok(prepared.program_requests.pop().unwrap())
}

/// Add the sponsor's real SIGHASH_ALL signature after the oracle accepts input zero.
pub fn sign_sponsor(psbt: &mut Psbt, key: &Keypair) -> Result<(), Error> {
    if psbt.inputs.len() != 2 || psbt.inputs[1].tap_internal_key != Some(key.x_only_public_key().0)
    {
        return Err("sponsor signing key does not match input one".into());
    }
    let prevouts: Vec<_> = psbt
        .inputs
        .iter()
        .map(|input| {
            input
                .witness_utxo
                .as_ref()
                .ok_or("missing authenticated prevout")
        })
        .collect::<Result<_, _>>()?;
    let hash_ty = TapSighashType::All;
    let hash = SighashCache::new(&psbt.unsigned_tx).taproot_key_spend_signature_hash(
        1,
        &Prevouts::All(&prevouts),
        hash_ty,
    )?;
    let secp = Secp256k1::new();
    let key = key.tap_tweak(&secp, None).to_keypair();
    psbt.inputs[1].tap_key_sig = Some(taproot::Signature {
        signature: secp.sign_schnorr_no_aux_rand(&Message::from_digest_slice(hash.as_ref())?, &key),
        sighash_type: hash_ty,
    });
    Ok(())
}
