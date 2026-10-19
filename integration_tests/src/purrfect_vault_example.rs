//! Disposable funding and explicit signing for the Purrfect vault example.

use bitcoin::bip32::{Xpriv, Xpub};
use bitcoin::hashes::{sha256, Hash};
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::Message;
use bitcoin::sighash::{Prevouts, SighashCache};
use bitcoin::{Address, Amount, CompressedPublicKey, EcdsaSighashType, Network, OutPoint};
use bitcoin::{ScriptBuf, Transaction, TxOut};
use emulator_connect::program::completion::SpendIntent;
use emulator_connect::program::{
    prepare_spend, ProgramCapability, ProgramEvidence, ProgramOracle, ProgramSpendPath,
    SpendAssets, SpendPath,
};
use sapio::contract::abi::object::ProgramRequirement;
use sapio::contract::*;
use sapio::template::Template;
use sapio_base::effects::MapEffectDB;
use sapio_base::util::CTVHash;
use sapio_contrib::contracts::purrfect_vault::{
    script_witness, synthetic_signature, PurrfectVault, Stage, WITHDRAWAL_DELAY,
};
use sapio_psbt::SigningKey;
use std::sync::Arc;

/// Errors are propagated to the executable or test boundary.
pub type Error = Box<dyn std::error::Error>;
/// The principal is preserved by every operation.
pub const PRINCIPAL_SATS: u64 = 100_000;
/// Each sponsor pays the marker, when present, and the remaining miner fee.
pub const SPONSOR_SATS: u64 = 2_000;

/// Published disposable keys; never use these fixtures with real funds.
pub fn owner_key() -> Xpriv {
    Xpriv::new_master(Network::Testnet, &[121; 32]).unwrap()
}
/// Published disposable oracle root.
pub fn oracle_key() -> Xpriv {
    Xpriv::new_master(Network::Testnet, &[122; 32]).unwrap()
}
/// Published disposable native witness-v0 fee key.
pub fn sponsor_key() -> Xpriv {
    Xpriv::new_master(Network::Testnet, &[123; 32]).unwrap()
}
/// All three public predicates use this explicit oracle root.
pub fn vault() -> Result<PurrfectVault, Error> {
    let secp = sapio_base::secp::secp();
    Ok(PurrfectVault::new(
        owner_key().to_keypair(secp).x_only_public_key().0,
        Xpub::from_priv(secp, &oracle_key()),
        Amount::from_sat(SPONSOR_SATS),
    )?)
}
/// A standard destination distinct from the fee sponsor.
pub fn destination(seed: u8) -> Address {
    crate::program_example::recipient(seed)
}
/// The sponsor is ordinary P2WPKH, excluding a second Taproot covenant input.
pub fn sponsor_address() -> Address {
    Address::p2wpkh(
        &CompressedPublicKey(
            sponsor_key()
                .to_keypair(sapio_base::secp::secp())
                .public_key(),
        ),
        Network::Regtest,
    )
}

fn context(effects: MapEffectDB) -> Result<Context, Error> {
    Ok(Context::new(
        Network::Regtest,
        Amount::from_sat(PRINCIPAL_SATS),
        sapio_base::LoweringPlan::Native,
        "purrfect_vault".try_into()?,
        Arc::new(effects),
        None,
    ))
}

/// Compile the funded policies without proposing a transaction.
pub fn compile(vault: &PurrfectVault) -> Result<Compiled, Error> {
    Ok(vault.compile(context(MapEffectDB::default())?)?)
}
/// Propose a trigger through Sapio's typed continuation interface.
pub fn compile_trigger(vault: &PurrfectVault, destination: ScriptBuf) -> Result<Compiled, Error> {
    let effects =
        PurrfectVault::trigger_action().request(&"purrfect_vault".try_into()?, &destination)?;
    Ok(vault.compile(context(effects)?)?)
}
/// Propose a cancellation without selecting another destination.
pub fn compile_cancel(vault: &PurrfectVault) -> Result<Compiled, Error> {
    let effects = PurrfectVault::cancel_action().request(&"purrfect_vault".try_into()?, &())?;
    Ok(vault.compile(context(effects)?)?)
}
/// Recover a completion proposal from the immediate parent transaction.
pub fn compile_complete(vault: &PurrfectVault, trigger: &Transaction) -> Result<Compiled, Error> {
    let effects =
        PurrfectVault::complete_action().request(&"purrfect_vault".try_into()?, trigger)?;
    Ok(vault.compile(context(effects)?)?)
}
/// Each requested action contributes exactly one non-CTV proposal.
pub fn candidate(compiled: &Compiled) -> Result<&Template, Error> {
    if compiled.suggested_txs.len() != 1 || !compiled.ctv_to_tx.is_empty() {
        return Err("expected one Purrfect vault continuation".into());
    }
    let template = compiled.suggested_txs.values().next().unwrap();
    if template.tx.input.len() != 2 || template.tx.output.is_empty() {
        return Err("expected two inputs and a principal output".into());
    }
    Ok(template)
}

/// A caller-supplied funding assertion; obtain actual coins from a node.
#[derive(Clone, Debug)]
pub struct Coin {
    /// The selected transaction output.
    pub outpoint: OutPoint,
    /// Caller-authenticated previous amount and script; signatures commit them.
    pub txout: TxOut,
}

fn synthetic_outpoint(tag: u8) -> OutPoint {
    OutPoint::new(
        bitcoin::Txid::from_byte_array(sha256::Hash::hash(&[tag]).to_byte_array()),
        0,
    )
}
/// Synthetic funding for the offline demonstration.
pub fn funding_coin(compiled: &Compiled) -> Coin {
    Coin {
        outpoint: synthetic_outpoint(1),
        txout: TxOut {
            value: Amount::from_sat(PRINCIPAL_SATS),
            script_pubkey: (&compiled.address).into(),
        },
    }
}
/// Link the next operation to the actual output created by its parent.
pub fn principal_coin(transaction: &Transaction) -> Coin {
    Coin {
        outpoint: OutPoint::new(transaction.compute_txid(), 0),
        txout: transaction.output[0].clone(),
    }
}
/// Synthetic fee sponsorship, replaceable with a funded outpoint by the node driver.
pub fn sponsor_coin(tag: u8) -> Coin {
    Coin {
        outpoint: synthetic_outpoint(tag),
        txout: TxOut {
            value: Amount::from_sat(SPONSOR_SATS),
            script_pubkey: sponsor_address().script_pubkey(),
        },
    }
}

/// Bind a proposed transaction to explicit principal and fee coins.
pub fn bind(
    compiled: &mut Compiled,
    vault: &PurrfectVault,
    stage: Stage,
    principal: Coin,
    sponsor: Coin,
) -> Result<Psbt, Error> {
    let mut template = candidate(compiled)?.clone();
    if principal.txout.script_pubkey != ScriptBuf::from(&compiled.address)
        || principal.txout.value != template.tx.output[0].value
        || sponsor.txout.script_pubkey != sponsor_address().script_pubkey()
        || principal.outpoint == sponsor.outpoint
    {
        return Err("funding must match the vault and independent witness-v0 sponsor".into());
    }
    let mut tx = template.tx.clone();
    tx.input[0].previous_output = principal.outpoint;
    tx.input[1].previous_output = sponsor.outpoint;
    let mut psbt = Psbt::from_unsigned_tx(tx)?;
    psbt.inputs[0].witness_utxo = Some(principal.txout);
    psbt.inputs[1].witness_utxo = Some(sponsor.txout);
    let ProgramSpendPath::ScriptPath(leaf) = requirement(compiled, vault, stage)?.path else {
        unreachable!()
    };
    // The CAT construction increments only the challenge's last byte. Grind
    // before any signatures exist, using a sponsor sequence with BIP68 disabled.
    // Retain the chosen proposal and its local fee rules in the saved artifact.
    for nonce in 0..4_096 {
        psbt.unsigned_tx.input[1].sequence = bitcoin::Sequence::from_consensus((1 << 31) | nonce);
        if synthetic_signature(&psbt, leaf)?.is_some() {
            template.tx.input[1].sequence = psbt.unsigned_tx.input[1].sequence;
            template.ctv = template.tx.get_ctv_hash(template.ctv_index);
            template.check_funded_psbt(&psbt)?;
            compiled.suggested_txs = [(template.ctv, template)].into_iter().collect();
            compiled.validate()?;
            return Ok(psbt);
        }
    }
    Err("could not grind a CAT signature within the attempt bound".into())
}

/// Select one exact script-path predicate from the retained artifact.
pub fn requirement(
    compiled: &Compiled,
    vault: &PurrfectVault,
    stage: Stage,
) -> Result<ProgramRequirement, Error> {
    let program = vault.program(stage)?;
    let mut matching = compiled
        .program_requirements()?
        .into_iter()
        .filter(|requirement| {
            requirement.program == program
                && matches!(requirement.path, ProgramSpendPath::ScriptPath(_))
        });
    let selected = matching.next().ok_or("missing vault program branch")?;
    if matching.next().is_some() {
        return Err("ambiguous vault program branch".into());
    }
    Ok(selected)
}

/// Retain a selected branch, its native owner slot, and exact oracle evidence.
pub fn prepare(
    compiled: &Compiled,
    vault: &PurrfectVault,
    stage: Stage,
    psbt: Psbt,
    parent: Option<&Transaction>,
) -> Result<SpendIntent, Error> {
    let requirement = requirement(compiled, vault, stage)?;
    let ProgramSpendPath::ScriptPath(leaf) = requirement.path else {
        unreachable!()
    };
    let prevouts: Vec<_> = psbt
        .inputs
        .iter()
        .map(|input| input.witness_utxo.clone().ok_or("missing prevout"))
        .collect::<Result<_, _>>()?;
    let hash_type = bitcoin::TapSighashType::All;
    let digest = SighashCache::new(&psbt.unsigned_tx).taproot_script_spend_signature_hash(
        0,
        &Prevouts::All(&prevouts),
        leaf,
        hash_type,
    )?;
    let owner = bitcoin::taproot::Signature {
        signature: sapio_base::secp::secp().sign_schnorr_no_aux_rand(
            &Message::from_digest(digest.to_byte_array()),
            &owner_key().to_keypair(sapio_base::secp::secp()),
        ),
        sighash_type: hash_type,
    };
    let stack = script_witness(stage, &psbt, leaf, parent, owner)?;
    let witness = sapio_base::bitcoin_script::encode_stack(&stack)?;
    let codec = "bitcoin-script/stack-v1";
    let assets = SpendAssets {
        schnorr_keys: [vault.owner()].into_iter().collect(),
        programs: vec![ProgramCapability {
            requirement: requirement.clone(),
            codec: codec.into(),
            evidence_available: true,
            signer_available: true,
        }],
        ..Default::default()
    };
    let evidence = ProgramEvidence {
        requirement,
        codec: codec.into(),
        witness,
    };
    Ok(SpendIntent::from_prepared(prepare_spend(
        compiled,
        SpendPath::ScriptPath(leaf),
        psbt,
        0,
        &assets,
        &[evidence],
    )?))
}

/// Sign the ordinary P2WPKH sponsor separately from the selected vault input.
pub fn sign_sponsor(psbt: &mut Psbt) -> Result<(), Error> {
    let prevout = psbt
        .inputs
        .get(1)
        .and_then(|input| input.witness_utxo.as_ref())
        .ok_or("missing fee sponsor")?;
    if prevout.script_pubkey != sponsor_address().script_pubkey() {
        return Err("fee sponsor key does not match prevout".into());
    }
    let sighash_type = EcdsaSighashType::All;
    let digest = SighashCache::new(&psbt.unsigned_tx).p2wpkh_signature_hash(
        1,
        &prevout.script_pubkey,
        prevout.value,
        sighash_type,
    )?;
    let key = sponsor_key();
    let secp = sapio_base::secp::secp();
    psbt.inputs[1].partial_sigs.insert(
        bitcoin::PublicKey::new(key.to_keypair(secp).public_key()),
        bitcoin::ecdsa::Signature {
            signature: secp.sign_ecdsa(
                &Message::from_digest(digest.to_byte_array()),
                &key.private_key,
            ),
            sighash_type,
        },
    );
    Ok(())
}

/// Obtain both independent authorizations, then finalize the selected branch.
pub fn sign(
    compiled: &Compiled,
    intent: &SpendIntent,
    oracle: &ProgramOracle,
) -> Result<Psbt, Error> {
    let mut current = intent.baseline_psbt().clone();
    for (index, request) in intent.requests().iter().enumerate() {
        let response = oracle.sign(request.clone())?;
        intent.merge_response(compiled, &mut current, index, &response)?;
    }
    intent.sign_native(
        compiled,
        &mut current,
        &SigningKey(vec![owner_key()]),
        sapio_base::secp::secp(),
    )?;
    sign_sponsor(&mut current)?;
    Ok(current)
}
/// Local finalization checks signatures and native transaction constraints.
pub fn finish(
    compiled: &Compiled,
    intent: &SpendIntent,
    oracle: &ProgramOracle,
) -> Result<Transaction, Error> {
    let signed = sign(compiled, intent, oracle)?;
    Ok(intent
        .finalize(compiled, &signed, sapio_base::secp::secp())?
        .extract_tx()?)
}

/// Finalize one typed action's candidate against the explicitly supplied coins.
pub fn execute(
    compiled: &Compiled,
    vault: &PurrfectVault,
    stage: Stage,
    principal: Coin,
    sponsor: Coin,
    previous_trigger: Option<&Transaction>,
    oracle: &ProgramOracle,
) -> Result<Transaction, Error> {
    let mut compiled = compiled.clone();
    let psbt = bind(&mut compiled, vault, stage, principal, sponsor)?;
    let intent = prepare(&compiled, vault, stage, psbt, previous_trigger)?;
    finish(&compiled, &intent, oracle)
}

/// Real outpoints supplied by the isolated node driver; all amounts/scripts
/// must match the manifest emitted by the offline run.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Funding {
    /// A confirmed vault deposit.
    pub funding: OutPoint,
    /// Four independent confirmed P2WPKH coins.
    pub sponsors: Vec<OutPoint>,
}

/// Run deposit -> trigger -> cancel -> trigger -> complete locally. Chain
/// maturity is checked separately by the node driver, never inferred here.
pub fn scenario(supplied: Option<Funding>) -> Result<serde_json::Value, Error> {
    use bitcoin::consensus::encode::serialize_hex;
    use serde_json::json;
    if let Some(funding) = &supplied {
        let distinct: std::collections::BTreeSet<_> = std::iter::once(funding.funding)
            .chain(funding.sponsors.iter().copied())
            .collect();
        if funding.sponsors.len() != 4 || distinct.len() != 5 {
            return Err(
                "the vault flow requires a deposit and four independent sponsor coins".into(),
            );
        }
    }
    let vault = vault()?;
    let bare = compile(&vault)?;
    let mut principal = funding_coin(&bare);
    let mut sponsors: Vec<_> = (2..6).map(sponsor_coin).collect();
    if let Some(funding) = supplied {
        principal.outpoint = funding.funding;
        for (coin, outpoint) in sponsors.iter_mut().zip(funding.sponsors) {
            coin.outpoint = outpoint;
        }
    }
    let oracle = ProgramOracle::new(oracle_key(), vec![])?;
    let first = compile_trigger(&vault, destination(124).script_pubkey())?;
    let trigger_cancel = execute(
        &first,
        &vault,
        Stage::Trigger,
        principal,
        sponsors[0].clone(),
        None,
        &oracle,
    )?;
    let cancellation = compile_cancel(&vault)?;
    let cancel = execute(
        &cancellation,
        &vault,
        Stage::Cancel,
        principal_coin(&trigger_cancel),
        sponsors[1].clone(),
        None,
        &oracle,
    )?;
    let payout = destination(125);
    let second = compile_trigger(&vault, payout.script_pubkey())?;
    let trigger_complete = execute(
        &second,
        &vault,
        Stage::Trigger,
        principal_coin(&cancel),
        sponsors[2].clone(),
        None,
        &oracle,
    )?;
    let mut completion = compile_complete(&vault, &trigger_complete)?;
    let psbt = bind(
        &mut completion,
        &vault,
        Stage::Complete,
        principal_coin(&trigger_complete),
        sponsors[3].clone(),
    )?;
    let intent = prepare(
        &completion,
        &vault,
        Stage::Complete,
        psbt,
        Some(&trigger_complete),
    )?;
    let signed = sign(&completion, &intent, &oracle)?;
    let requirement = requirement(&completion, &vault, Stage::Complete)?;
    let ProgramSpendPath::ScriptPath(leaf) = requirement.path else {
        unreachable!()
    };
    let owner_signature = signed.inputs[0]
        .tap_script_sigs
        .get(&(vault.owner(), leaf))
        .ok_or("missing owner authorization")?
        .to_vec();
    let complete = intent
        .finalize(&completion, &signed, sapio_base::secp::secp())?
        .extract_tx()?;
    let mut changed = complete.clone();
    changed.output[0].value -= Amount::ONE_SAT;
    let mut missing_owner = complete.clone();
    let mut witness = missing_owner.input[0].witness.to_vec();
    let owner_slot = witness
        .iter_mut()
        .find(|element| **element == owner_signature)
        .ok_or("missing native owner witness")?;
    owner_slot.clear();
    missing_owner.input[0].witness = bitcoin::Witness::from_slice(&witness);
    Ok(json!({
        "enforcement": "CAT Script executed by WASM interpreter; native owner signatures and CSV",
        "funding": {"address": Address::from_script(&ScriptBuf::from(&bare.address), Network::Regtest)?.to_string(), "amount_sats": PRINCIPAL_SATS},
        "sponsor": {"address": sponsor_address().to_string(), "amount_sats": SPONSOR_SATS, "count": 4},
        "delay_blocks": WITHDRAWAL_DELAY,
        "destination": {"address": payout.to_string(), "amount_sats": PRINCIPAL_SATS},
        "transactions": {
            "trigger_cancel": serialize_hex(&trigger_cancel),
            "cancel": serialize_hex(&cancel),
            "trigger_complete": serialize_hex(&trigger_complete),
            "complete": serialize_hex(&complete),
            "complete_changed_after_signing": serialize_hex(&changed),
            "complete_without_owner": serialize_hex(&missing_owner),
        },
    }))
}
