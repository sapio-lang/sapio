//! A disposable runner for the depot protocol contract in `sapio-contrib`.
//!
//! Contract source lives in [`sapio_contrib::contracts::depots`]. This module
//! assembles regtest compilation, channel purchases (the operator's blind
//! guess signatures), target reveals, and authenticated spending requests
//! against the program oracle running the depots evaluator. The fixture's
//! keys are disposable; the salt is random bytes rather than the paper's
//! joint signature, and one oracle stands in for a federation.

use bitcoin::bip32::{Xpriv, Xpub};
use bitcoin::hashes::{sha256, Hash};
use bitcoin::key::TapTweak;
use bitcoin::psbt::{Input, Psbt};
use bitcoin::secp256k1::{Keypair, Message, Secp256k1, SecretKey};
use bitcoin::sighash::{Prevouts, SighashCache};
use bitcoin::{
    taproot, Address, Amount, Network, OutPoint, ScriptBuf, TapSighashType, Transaction, TxOut,
    XOnlyPublicKey,
};
use emulator_connect::program::completion::SpendIntent;
use emulator_connect::program::spend_plan::{ProgramCapability, ProgramEvidence, SpendPath};
use emulator_connect::program::{prepare_spend, ProgramOracle, ProgramSpendPath, SpendAssets};
use sapio::contract::abi::object::ProgramRequirement;
use sapio::contract::{Compilable, Compiled, Context};
use sapio::template::Template;
use sapio_base::effects::MapEffectDB;
use sapio_base::fragments::{template_hash, templatehash_wasm_instance};
use sapio_base::program::ProgramInstance;
use sapio_base::timelocks::AbsHeight;
use sapio_contrib::contracts::depots::{
    target_evidence, ChannelBalance, ClaimRequest, DepotTerms, GuessTuple, ResolutionMode, Stage,
};
use std::convert::TryFrom;
use std::sync::Arc;

/// Errors in the example's compilation, binding and signing helpers.
pub type Error = Box<dyn std::error::Error>;

/// Sample depot value `D`, in satoshis. A hit funds a `D/2` channel.
pub const DEPOT_SATS: u64 = 100_000;
/// Sample randomization prime `P`.
pub const PRIME: u64 = 5;
/// Sample depot expiry height `E`.
pub const EXPIRY: u32 = 100_000;
/// Sample monitoring window `tsd`, in blocks.
pub const TSD: u16 = 6;

fn key(seed: u8) -> Keypair {
    Keypair::from_secret_key(
        sapio_base::secp::secp(),
        &SecretKey::from_slice(&[seed; 32]).unwrap(),
    )
}
/// The operator's payout key for sweeps and settlements.
pub fn operator_key() -> Keypair {
    key(110)
}
/// The key whose signature reveals the depot's target.
pub fn target_key() -> Keypair {
    key(111)
}
/// The key whose blind signatures sell channels.
pub fn guess_key() -> Keypair {
    key(112)
}
/// Replaceable fee sponsor key.
pub fn sponsor_key() -> Keypair {
    key(113)
}
/// A depot user's key.
pub fn user_key(seed: u8) -> Keypair {
    key(120 + seed)
}
/// The emulation oracle's private root.
pub fn oracle_key() -> Xpriv {
    Xpriv::new_master(Network::Testnet, &[114; 32]).unwrap()
}
/// The oracle running the inline depots evaluator; nothing to register.
pub fn oracle() -> ProgramOracle {
    ProgramOracle::new(oracle_key(), vec![]).unwrap()
}

/// Sample depot terms bound to the fixture keys and oracle.
pub fn terms() -> DepotTerms {
    DepotTerms::new(
        operator_key().x_only_public_key().0,
        target_key().x_only_public_key().0,
        guess_key().x_only_public_key().0,
        Xpub::from_priv(sapio_base::secp::secp(), &oracle_key()),
        Amount::from_sat(DEPOT_SATS),
        PRIME,
        AbsHeight::try_from(EXPIRY).unwrap(),
        TSD,
        Amount::from_sat(10_000),
    )
    .unwrap()
}

fn context(amount: Amount) -> Result<Context, Error> {
    Ok(Context::new(
        Network::Regtest,
        amount,
        sapio_base::LoweringPlan::Native,
        "depots".try_into()?,
        Arc::new(Default::default()),
        None,
    ))
}

fn compile_with<C: Compilable>(
    source: &C,
    amount: Amount,
    effects: MapEffectDB,
) -> Result<Compiled, Error> {
    Ok(source.compile(Context::new(
        Network::Regtest,
        amount,
        sapio_base::LoweringPlan::Native,
        "depots".try_into()?,
        Arc::new(effects),
        None,
    ))?)
}

/// Compile the depot funding output with one resolution branch's candidate.
pub fn compile_funding(terms: &DepotTerms, mode: ResolutionMode) -> Result<Compiled, Error> {
    use sapio_contrib::contracts::depots::Depot;
    let effects = match mode {
        ResolutionMode::Operator => Depot::reveal_target_action().request(&path()?, &())?,
        ResolutionMode::Default => Depot::default_target_action().request(&path()?, &())?,
    };
    compile_with(&terms.funding(), terms.value(), effects)
}

/// Compile the Target transaction's output with its Hit candidate.
pub fn compile_target_output(terms: &DepotTerms, mode: ResolutionMode) -> Result<Compiled, Error> {
    use sapio_contrib::contracts::depots::TargetOutput;
    compile_with(
        &terms.target_output(mode),
        terms.value(),
        TargetOutput::hit_action().request(&path()?, &())?,
    )
}

/// Compile Hit output 1 with a specific channel's claim candidate.
pub fn compile_hit_claim(
    terms: &DepotTerms,
    mode: ResolutionMode,
    user: XOnlyPublicKey,
) -> Result<Compiled, Error> {
    use sapio_contrib::contracts::depots::HitClaimOutput;
    compile_with(
        &terms.hit_claim_output(mode),
        terms.value() / 2,
        HitClaimOutput::claim_action().request(&path()?, &ClaimRequest { user })?,
    )
}

/// Compile Hit output 1 with its burn candidate.
pub fn compile_hit_claim_burn(terms: &DepotTerms, mode: ResolutionMode) -> Result<Compiled, Error> {
    use sapio_contrib::contracts::depots::HitClaimOutput;
    compile_with(
        &terms.hit_claim_output(mode),
        terms.value() / 2,
        HitClaimOutput::burn_action().request(&path()?, &())?,
    )
}

/// Compile Hit output 0 with its burn candidate.
pub fn compile_hit_insurance_burn(
    terms: &DepotTerms,
    mode: ResolutionMode,
) -> Result<Compiled, Error> {
    use sapio_contrib::contracts::depots::HitInsuranceOutput;
    compile_with(
        &terms.hit_insurance_output(mode),
        terms.value() / 2,
        HitInsuranceOutput::burn_action().request(&path()?, &())?,
    )
}

/// Compile a claimed channel's settlement layer with a final balance.
pub fn compile_claim_output(
    terms: &DepotTerms,
    mode: ResolutionMode,
    user: XOnlyPublicKey,
    balance: ChannelBalance,
) -> Result<Compiled, Error> {
    use sapio_contrib::contracts::depots::ClaimOutput;
    compile_with(
        &terms.claim_output(user, mode),
        terms.value() / 2,
        ClaimOutput::commit_action().request(&path()?, &balance)?,
    )
}

fn path() -> Result<sapio_base::effects::EffectPath, Error> {
    Ok("depots".try_into()?)
}

/// The single transition candidate of a compiled artifact.
pub fn only_candidate(compiled: &Compiled) -> Result<&Template, Error> {
    if compiled.suggested_txs.len() != 1 || !compiled.ctv_to_tx.is_empty() {
        return Err("expected exactly one continuation candidate and no CTV branches".into());
    }
    Ok(compiled.suggested_txs.values().next().unwrap())
}

fn requirement_for(
    compiled: &Compiled,
    instance: &ProgramInstance,
) -> Result<ProgramRequirement, Error> {
    let requirements = compiled.program_requirements()?;
    let mut matching = requirements.into_iter().filter(|requirement| {
        requirement.program.instance() == instance
            && matches!(requirement.path, ProgramSpendPath::ScriptPath(_))
    });
    let requirement = matching
        .next()
        .ok_or("artifact has no matching depot program path")?;
    if matching.next().is_some() {
        return Err("artifact has ambiguous depot program paths".into());
    }
    Ok(requirement)
}

/// Select the artifact's depots-evaluator leaf for one stage of resolution.
pub fn depot_requirement(
    compiled: &Compiled,
    terms: &DepotTerms,
    stage: Stage,
    mode: ResolutionMode,
    expected: sha256::Hash,
) -> Result<ProgramRequirement, Error> {
    let program = terms.program(stage, mode, expected)?;
    requirement_for(compiled, program.instance())
}

/// Select the funding artifact's default-target templatehash leaf.
pub fn default_target_requirement(
    compiled: &Compiled,
    template: &Template,
) -> Result<ProgramRequirement, Error> {
    requirement_for(
        compiled,
        &templatehash_wasm_instance(template_hash(&template.tx, 0, None)?),
    )
}

/// The BIP446 commitment of a candidate, as the evaluator checks it.
pub fn commitment(template: &Template) -> Result<sha256::Hash, Error> {
    Ok(template_hash(&template.tx, 0, None)?)
}

/// A funding assertion; the caller must obtain actual prevouts from its node.
#[derive(Clone, Debug)]
pub struct Coin {
    /// The spent outpoint.
    pub outpoint: OutPoint,
    /// Its authenticated prevout.
    pub txout: TxOut,
}

/// Replaceable key-path sponsor; its entire value becomes the fee.
#[derive(Clone, Debug)]
pub struct Sponsor {
    /// The sponsor coin.
    pub coin: Coin,
    /// The untweaked P2TR internal key holding it.
    pub internal_key: XOnlyPublicKey,
}

fn outpoint(tag: u8) -> OutPoint {
    OutPoint::new(
        bitcoin::Txid::from_byte_array(sha256::Hash::hash(&[tag]).to_byte_array()),
        0,
    )
}

/// A synthetic coin holding a compiled artifact's exact output.
pub fn coin_for(compiled: &Compiled, value: Amount, tag: u8) -> Coin {
    Coin {
        outpoint: outpoint(tag),
        txout: TxOut {
            value,
            script_pubkey: (&compiled.address).into(),
        },
    }
}

/// A coin created by an actual prior transaction in the flow.
pub fn coin_from(transaction: &Transaction, vout: u32) -> Coin {
    Coin {
        outpoint: OutPoint::new(transaction.compute_txid(), vout),
        txout: transaction.output[vout as usize].clone(),
    }
}

/// A synthetic sponsor coin.
pub fn sponsor(value: u64, tag: u8) -> Sponsor {
    let internal_key = sponsor_key().x_only_public_key().0;
    Sponsor {
        coin: Coin {
            outpoint: outpoint(tag),
            txout: TxOut {
                value: Amount::from_sat(value),
                script_pubkey: Address::p2tr(
                    sapio_base::secp::secp(),
                    internal_key,
                    None,
                    Network::Regtest,
                )
                .script_pubkey(),
            },
        },
        internal_key,
    }
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

/// Attach a verified depot coin to the compiler's spending proof.
pub fn input_from_artifact(compiled: &Compiled, coin: &Coin) -> Result<Input, Error> {
    if coin.txout.script_pubkey != ScriptBuf::from(&compiled.address) {
        return Err("depot funding does not match the compiled output".into());
    }
    let mut input = spending_input(compiled)?;
    input.witness_utxo = Some(coin.txout.clone());
    Ok(input)
}

/// Bind an authorized template to authenticated contract and sponsor coins.
pub fn attach_inputs(
    mut transaction: Transaction,
    contract: Coin,
    mut input: Input,
    sponsor: Sponsor,
) -> Result<Psbt, Error> {
    if transaction.version != bitcoin::transaction::Version::TWO
        || transaction.input.len() != 2
        || sponsor.coin.txout.value.to_sat() == 0
    {
        return Err("depots require version two, two inputs and a positive sponsor".into());
    }
    let script = Address::p2tr(
        sapio_base::secp::secp(),
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
    if contract.txout.value.to_sat() != output_sum
        || input.witness_utxo.as_ref() != Some(&contract.txout)
    {
        return Err("depot value and authenticated prevout must match all outputs".into());
    }
    transaction.input[0].previous_output = contract.outpoint;
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

/// A channel purchase: the user's secret guess and salt, plus its index.
#[derive(Clone, Debug)]
pub struct Purchase {
    /// The purchasing user's key.
    pub user: XOnlyPublicKey,
    /// The channel's index within the depot.
    pub index: u64,
    /// The salt hiding the guess inside the blind-signed message.
    pub salt: [u8; 32],
    /// The channel's secret guess in `0..PRIME`.
    pub guess: u64,
}

impl Purchase {
    /// A deterministic sample purchase.
    pub fn new(user_seed: u8, index: u64, guess: u64) -> Self {
        Self {
            user: user_key(user_seed).x_only_public_key().0,
            index,
            salt: sha256::Hash::hash(&[user_seed, 0x53]).to_byte_array(),
            guess,
        }
    }
}

/// The operator's target signature TaSig for `t`.
pub fn sign_target(terms: &DepotTerms, target: u64) -> [u8; 64] {
    let message = terms.target_message(target);
    *Secp256k1::new()
        .sign_schnorr_no_aux_rand(&Message::from_digest(message), &target_key())
        .as_ref()
}

/// Sell a channel: bind its claim transactions and blind-sign the guess.
/// The real protocol signs blind via an adaptor over a PairCommit chain;
/// the fixture operator signs the committed message directly.
pub fn purchase_channel(terms: &DepotTerms, purchase: &Purchase) -> Result<GuessTuple, Error> {
    let (clm, clm_prime) = terms.claim_hashes(context(terms.value() / 2)?, purchase.user)?;
    let message = terms.guess_message(
        clm,
        clm_prime,
        purchase.index,
        purchase.salt,
        purchase.guess,
    );
    let signature = *Secp256k1::new()
        .sign_schnorr_no_aux_rand(&Message::from_digest(message), &guess_key())
        .as_ref();
    Ok(GuessTuple {
        index: purchase.index,
        salt: purchase.salt,
        guess: purchase.guess,
        signature,
        clm,
        clm_prime,
    })
}

/// Evidence for one-tuple stages: the target section then the guess tuple.
pub fn hit_evidence(
    mode: ResolutionMode,
    target: Option<(u64, [u8; 64])>,
    tuple: &GuessTuple,
) -> Vec<u8> {
    let mut witness = match (mode, target) {
        (ResolutionMode::Operator, Some((t, signature))) => target_evidence(t, &signature),
        _ => vec![],
    };
    witness.extend_from_slice(&tuple.encode());
    witness
}

/// Evidence for the double-hit burn: the target section then both tuples.
pub fn burn_evidence(
    mode: ResolutionMode,
    target: Option<(u64, [u8; 64])>,
    first: &GuessTuple,
    second: &GuessTuple,
) -> Vec<u8> {
    let mut witness = hit_evidence(mode, target, first);
    witness.extend_from_slice(&second.encode());
    witness
}

/// Prepare one program-guarded branch with explicit evidence.
pub fn prepare_depot_spend(
    compiled: &Compiled,
    requirement: ProgramRequirement,
    psbt: Psbt,
    codec: &str,
    witness: Vec<u8>,
) -> Result<SpendIntent, Error> {
    let ProgramSpendPath::ScriptPath(leaf) = requirement.path else {
        return Err("depot authorization must use a script path".into());
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
    let prepared = prepare_spend(
        compiled,
        SpendPath::ScriptPath(leaf),
        psbt,
        0,
        &assets,
        &evidence,
    )?;
    Ok(SpendIntent::from_prepared(prepared))
}

/// Obtain the oracle and sponsor signatures, then complete the branch.
pub fn complete_candidate(
    compiled: &Compiled,
    intent: &SpendIntent,
    oracle: &ProgramOracle,
) -> Result<Transaction, Error> {
    let mut current = intent.baseline_psbt().clone();
    intent.status(compiled, &current)?;
    for (index, request) in intent.requests().iter().enumerate() {
        let response = oracle.sign(request.clone())?;
        intent.merge_response(compiled, &mut current, index, &response)?;
    }
    sign_sponsor(&mut current, &sponsor_key())?;
    let finalized = intent.finalize(compiled, &current, sapio_base::secp::secp())?;
    Ok(finalized.extract_tx()?)
}

/// Ask the oracle to authorize one stage without completing the branch;
/// returns the oracle's error for predicate-rejection tests.
pub fn oracle_signs(
    compiled: &Compiled,
    intent: &SpendIntent,
    oracle: &ProgramOracle,
) -> Result<(), Error> {
    let mut current = intent.baseline_psbt().clone();
    for (index, request) in intent.requests().iter().enumerate() {
        let response = oracle.sign(request.clone())?;
        intent.merge_response(compiled, &mut current, index, &response)?;
    }
    Ok(())
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
    let secp = sapio_base::secp::secp();
    let key = key.tap_tweak(&secp, None).to_keypair();
    psbt.inputs[1].tap_key_sig = Some(taproot::Signature {
        signature: secp.sign_schnorr_no_aux_rand(&Message::from_digest_slice(hash.as_ref())?, &key),
        sighash_type: hash_ty,
    });
    Ok(())
}
