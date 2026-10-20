//! Signed LayerTree transitions with disposable keys and explicit fee coins.

use bitcoin::bip32::{Xpriv, Xpub};
use bitcoin::hashes::{sha256, Hash};
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::{Keypair, Message};
use bitcoin::sighash::{Prevouts, SighashCache};
use bitcoin::taproot::TapLeafHash;
use bitcoin::{Address, Amount, Network, OutPoint, ScriptBuf, Transaction, TxIn, TxOut};
use emulator_connect::program::completion::SpendIntent;
use emulator_connect::program::{
    prepare_spend, ProgramCapability, ProgramEvidence, ProgramOracle, ProgramSpendPath,
    SpendAssets, SpendPath,
};
use sapio::contract::{Compiled, Context};
use sapio_base::effects::MapEffectDB;
use sapio_base::secp::secp;
use sapio_contrib::contracts::layertree::{
    script_tree_root, tree, witness, Config, LayerTree, Leaf, Transition, Tree,
};
use std::sync::Arc;

pub type Error = Box<dyn std::error::Error>;
pub const FEE_SATS: u64 = 2_000;
pub const LEASE: u32 = 200;

/// Published example keys, with no production secrets.
pub fn key(seed: u8) -> Keypair {
    Xpriv::new_master(Network::Testnet, &[seed; 32])
        .unwrap()
        .to_keypair(secp())
}
pub fn oracle_key() -> Xpriv {
    Xpriv::new_master(Network::Testnet, &[202; 32]).unwrap()
}
pub fn oracle() -> Result<ProgramOracle, Error> {
    Ok(ProgramOracle::new(oracle_key(), vec![])?)
}
pub fn operator() -> Keypair {
    key(203)
}

pub fn contract() -> Result<LayerTree, Error> {
    Ok(LayerTree::new(
        Config {
            depth: 4,
            operator: operator().x_only_public_key().0,
            tree_id: tree::tree_id(outpoint(200)),
            dust_min: 546,
        },
        Xpub::from_priv(secp(), &oracle_key()),
    )?)
}

pub fn member(seed: u8, amount: u64) -> Leaf {
    Leaf {
        amount,
        exit_key: key(seed).x_only_public_key().0,
        owner_key: key(seed).x_only_public_key().0,
        evictor_key: operator().x_only_public_key().0,
        evict_after: LEASE,
        nonce: u32::from(seed),
        seq: 0,
    }
}
pub fn initial_tree() -> Result<Tree, Error> {
    let mut tree = Tree::new(4)?;
    for (position, record) in [
        (0, member(210, 60_000)),
        (1, member(211, 40_000)),
        (2, member(212, 20_000)),
    ] {
        tree.set(position, Some(record))?;
    }
    Ok(tree)
}
pub fn compile(contract: &LayerTree, tree: &Tree) -> Result<Compiled, Error> {
    let object = contract.compile_state(
        Context::new(
            Network::Regtest,
            Amount::from_sat(tree.root().sum),
            sapio_base::LoweringPlan::Native,
            "layertree".try_into()?,
            Arc::new(MapEffectDB::default()),
            None,
        ),
        tree,
    )?;
    // Replaying the portable artifact must retain the NUMS opening, complete
    // interpreter programs and the actual native script-tree commitment.
    let restored: Compiled = serde_json::from_slice(&serde_json::to_vec(&object)?)?;
    restored.validate()?;
    Ok(restored)
}

#[derive(Clone, Debug)]
pub struct Coin {
    pub outpoint: OutPoint,
    pub txout: TxOut,
}
pub fn outpoint(tag: u8) -> OutPoint {
    OutPoint::new(
        bitcoin::Txid::from_byte_array(sha256::Hash::hash(&[tag]).to_byte_array()),
        0,
    )
}
pub fn sponsor_address() -> Address {
    crate::purrfect_vault_example::sponsor_address()
}
pub fn payout(record: &Leaf) -> TxOut {
    TxOut {
        value: Amount::from_sat(record.amount),
        script_pubkey: ScriptBuf::from_bytes(
            [&[0x51, 0x20][..], &record.exit_key.serialize()].concat(),
        ),
    }
}
pub fn sign_transaction(psbt: &Psbt, leaf: TapLeafHash, key: &Keypair) -> Result<Vec<u8>, Error> {
    let prevouts: Vec<_> = psbt
        .inputs
        .iter()
        .map(|input| input.witness_utxo.clone().ok_or("missing prevout"))
        .collect::<Result<_, _>>()?;
    let digest = SighashCache::new(&psbt.unsigned_tx).taproot_script_spend_signature_hash(
        0,
        &Prevouts::All(&prevouts),
        leaf,
        bitcoin::TapSighashType::All,
    )?;
    let mut signature = secp()
        .sign_schnorr_no_aux_rand(&Message::from_digest(digest.to_byte_array()), key)
        .as_ref()
        .to_vec();
    signature.push(1);
    Ok(signature)
}
pub fn sign_voucher(message: [u8; 32], key: &Keypair) -> Vec<u8> {
    secp()
        .sign_schnorr_no_aux_rand(&Message::from_digest(message), key)
        .as_ref()
        .to_vec()
}

pub struct Step {
    pub compiled: Compiled,
    pub intent: SpendIntent,
    pub next: Tree,
    pub sponsor: Coin,
}

/// Prepare one transition. Fee ownership is checked locally; all membership,
/// authorization, conservation and successor-state rules execute in Script.
#[allow(clippy::too_many_arguments)] // Keep each fixture's coin and transaction fields explicit.
pub fn prepare_step(
    contract: &LayerTree,
    current: &Tree,
    principal: Coin,
    transition: Transition,
    payouts: Vec<TxOut>,
    lock_time: u32,
    sponsor_tag: u8,
    plan: impl Fn(&dyn Fn(&Keypair) -> Vec<u8>, [u8; 32]) -> Result<witness::Prepared, &'static str>,
) -> Result<Step, Error> {
    prepare_step_funded(
        contract,
        current,
        principal,
        transition,
        payouts,
        lock_time,
        sponsor_tag,
        None,
        plan,
    )
}

#[allow(clippy::too_many_arguments)]
fn prepare_step_funded(
    contract: &LayerTree,
    current: &Tree,
    principal: Coin,
    transition: Transition,
    payouts: Vec<TxOut>,
    lock_time: u32,
    sponsor_tag: u8,
    sponsor_outpoint: Option<OutPoint>,
    plan: impl Fn(&dyn Fn(&Keypair) -> Vec<u8>, [u8; 32]) -> Result<witness::Prepared, &'static str>,
) -> Result<Step, Error> {
    let compiled = compile(contract, current)?;
    if principal.txout
        != (TxOut {
            value: Amount::from_sat(current.root().sum),
            script_pubkey: (&compiled.address).into(),
        })
    {
        return Err("principal coin does not match the committed state".into());
    }
    let merkle = script_tree_root(&compiled)?;
    let (next, _) = plan(&|_| vec![], merkle)?;
    let successor = if next.root().sum == 0 {
        if next.root() != tree::Node::empty(next.depth()) {
            return Err("terminal state is not the empty tree".into());
        }
        // MID's terminal branch permits slot zero to be ordinary change.
        // A zero-valued OP_RETURN keeps payout indices and is standard.
        TxOut {
            value: Amount::ZERO,
            script_pubkey: ScriptBuf::from_bytes(vec![0x6a]),
        }
    } else {
        let next_compiled = compile(contract, &next)?;
        if script_tree_root(&next_compiled)? != merkle {
            return Err("successor changed the script set".into());
        }
        TxOut {
            value: Amount::from_sat(next.root().sum),
            script_pubkey: (&next_compiled.address).into(),
        }
    };
    let output_total = payouts
        .iter()
        .try_fold(next.root().sum, |sum, output| {
            sum.checked_add(output.value.to_sat())
        })
        .ok_or("output sum overflow")?;
    let sponsor_value = output_total
        .checked_add(FEE_SATS)
        .and_then(|sum| sum.checked_sub(current.root().sum))
        .ok_or("transition consumes principal as fees")?;
    let sponsor = Coin {
        outpoint: sponsor_outpoint.unwrap_or_else(|| outpoint(sponsor_tag)),
        txout: TxOut {
            value: Amount::from_sat(sponsor_value),
            script_pubkey: sponsor_address().script_pubkey(),
        },
    };
    if sponsor.outpoint == principal.outpoint {
        return Err("sponsor must be independent".into());
    }
    let mut outputs = vec![successor];
    outputs.extend(payouts);
    let mut psbt = Psbt::from_unsigned_tx(Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::absolute::LockTime::from_consensus(lock_time),
        input: [principal.outpoint, sponsor.outpoint]
            .map(|previous_output| TxIn {
                previous_output,
                sequence: bitcoin::Sequence::ENABLE_LOCKTIME_NO_RBF,
                ..Default::default()
            })
            .to_vec(),
        output: outputs,
    })?;
    psbt.inputs[0].witness_utxo = Some(principal.txout);
    psbt.inputs[1].witness_utxo = Some(sponsor.txout.clone());
    let program = contract.program(transition)?;
    let requirement = compiled
        .program_requirements()?
        .into_iter()
        .find(|item| item.program == program)
        .ok_or("missing transition program")?;
    let ProgramSpendPath::ScriptPath(leaf) = requirement.path else {
        return Err("LayerTree must use a script path".into());
    };
    // All keys share the same BIP341 message, so compute it once before the
    // pure witness constructor requests signatures for its authority slots.
    let prevouts: Vec<_> = psbt
        .inputs
        .iter()
        .map(|input| input.witness_utxo.clone().unwrap())
        .collect();
    let digest = SighashCache::new(&psbt.unsigned_tx).taproot_script_spend_signature_hash(
        0,
        &Prevouts::All(&prevouts),
        leaf,
        bitcoin::TapSighashType::All,
    )?;
    let sign = |key: &Keypair| {
        let mut bytes = secp()
            .sign_schnorr_no_aux_rand(&Message::from_digest(digest.to_byte_array()), key)
            .as_ref()
            .to_vec();
        bytes.push(1);
        bytes
    };
    let (signed_next, stack) = plan(&sign, merkle)?;
    if signed_next.root() != next.root() {
        return Err("authorization changed the proposed state".into());
    }
    let evidence = ProgramEvidence {
        requirement: requirement.clone(),
        codec: "bitcoin-script/stack-v1".into(),
        witness: sapio_base::bitcoin_script::encode_stack(&stack)?,
    };
    let assets = SpendAssets {
        programs: vec![ProgramCapability {
            requirement,
            codec: evidence.codec.clone(),
            evidence_available: true,
            signer_available: true,
        }],
        ..Default::default()
    };
    let intent = SpendIntent::from_prepared(prepare_spend(
        &compiled,
        SpendPath::ScriptPath(leaf),
        psbt,
        0,
        &assets,
        &[evidence],
    )?);
    let restored: SpendIntent = serde_json::from_slice(&serde_json::to_vec(&intent)?)?;
    restored.status(&compiled, restored.baseline_psbt())?;
    Ok(Step {
        compiled,
        intent: restored,
        next,
        sponsor,
    })
}

pub fn finish(step: &Step, oracle: &ProgramOracle) -> Result<Transaction, Error> {
    let mut psbt = step.intent.baseline_psbt().clone();
    for (index, request) in step.intent.requests().iter().enumerate() {
        let response = oracle.sign(request.clone())?;
        step.intent
            .merge_response(&step.compiled, &mut psbt, index, &response)?;
    }
    crate::purrfect_vault_example::sign_sponsor(&mut psbt)?;
    Ok(step
        .intent
        .finalize(&step.compiled, &psbt, secp())?
        .extract_tx()?)
}

/// The initial unilateral-exit case is also used to check rejected requests.
pub fn exit_fixture() -> Result<Step, Error> {
    let contract = contract()?;
    let tree = initial_tree()?;
    let compiled = compile(&contract, &tree)?;
    let principal = Coin {
        outpoint: outpoint(1),
        txout: TxOut {
            value: Amount::from_sat(tree.root().sum),
            script_pubkey: (&compiled.address).into(),
        },
    };
    prepare_step(
        &contract,
        &tree,
        principal,
        Transition::Exit { count: 1 },
        vec![payout(tree.leaf(2).unwrap())],
        0,
        2,
        |sign, merkle| witness::exit(&tree, &[(2, false, sign(&key(212)))], merkle),
    )
}

fn advance(
    name: &str,
    step: Step,
    oracle: &ProgramOracle,
    current: &mut Tree,
    principal: &mut Coin,
    rows: &mut Vec<serde_json::Value>,
) -> Result<(), Error> {
    let transaction = finish(&step, oracle)?;
    rows.push(serde_json::json!({
        "operation": name,
        "txid": transaction.compute_txid(),
        "transaction": bitcoin::consensus::encode::serialize_hex(&transaction),
        "old_principal": principal.txout.value.to_sat(),
        "new_principal": step.next.root().sum,
        "sponsor_sats": step.sponsor.txout.value.to_sat(),
        "fee_sats": FEE_SATS,
        "leaf_count": step.next.leaves().count(),
    }));
    *principal = Coin {
        outpoint: OutPoint::new(transaction.compute_txid(), 0),
        txout: transaction.output[0].clone(),
    };
    *current = step.next;
    Ok(())
}

/// Join, graft, update, split, redeem vouchers, exit and sweep linked states.
pub fn scenario() -> Result<serde_json::Value, Error> {
    scenario_funded(None)
}

/// Confirmed deposit and fee coins supplied by the isolated node driver.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Funding {
    pub funding: OutPoint,
    pub sponsors: Vec<OutPoint>,
}

pub fn scenario_funded(funding: Option<Funding>) -> Result<serde_json::Value, Error> {
    if let Some(funding) = &funding {
        let distinct: std::collections::BTreeSet<_> = std::iter::once(funding.funding)
            .chain(funding.sponsors.iter().copied())
            .collect();
        if funding.sponsors.len() != 11 || distinct.len() != 12 {
            return Err("expected one deposit and eleven independent sponsor coins".into());
        }
    }
    let sponsor_point = |index: usize| funding.as_ref().map(|funding| funding.sponsors[index]);
    let contract = contract()?;
    let oracle = oracle()?;
    let mut current = initial_tree()?;
    let initial = compile(&contract, &current)?;
    let deposit = initial.address.clone();
    let mut principal = Coin {
        outpoint: funding
            .as_ref()
            .map_or_else(|| outpoint(1), |funding| funding.funding),
        txout: TxOut {
            value: Amount::from_sat(current.root().sum),
            script_pubkey: (&initial.address).into(),
        },
    };
    let mut rows = Vec::new();

    let step = prepare_step_funded(
        &contract,
        &current,
        principal.clone(),
        Transition::Join { height: 0 },
        vec![],
        0,
        10,
        sponsor_point(0),
        |sign, m| {
            witness::join(
                &current,
                0,
                3,
                &[(3, member(213, 30_000))],
                &sign(&operator()),
                m,
            )
        },
    )?;
    advance(
        "join",
        step,
        &oracle,
        &mut current,
        &mut principal,
        &mut rows,
    )?;

    let cohort = [(8, member(218, 12_000)), (9, member(219, 8_000))];
    let step = prepare_step_funded(
        &contract,
        &current,
        principal.clone(),
        Transition::Join { height: 2 },
        vec![],
        0,
        11,
        sponsor_point(1),
        |sign, m| witness::join(&current, 2, 2, &cohort, &sign(&operator()), m),
    )?;
    advance(
        "graft",
        step,
        &oracle,
        &mut current,
        &mut principal,
        &mut rows,
    )?;

    let mut left = current.leaf(8).unwrap().clone();
    let mut right = current.leaf(9).unwrap().clone();
    left.amount = 10_000;
    right.amount = 10_000;
    let step = prepare_step_funded(
        &contract,
        &current,
        principal.clone(),
        Transition::Rewrite { height: 2 },
        vec![],
        0,
        20,
        sponsor_point(2),
        |sign, m| {
            witness::rewrite(
                &current,
                witness::Rewrite {
                    index: 2,
                    new: [Some(left.clone()), Some(right.clone()), None, None],
                    signatures: [&sign(&key(218)), &[], &[], &[]],
                },
                &sign(&operator()),
                m,
            )
        },
    )?;
    advance(
        "rewrite",
        step,
        &oracle,
        &mut current,
        &mut principal,
        &mut rows,
    )?;

    let mut updated = current.leaf(1).unwrap().clone();
    updated.owner_key = key(221).x_only_public_key().0;
    let step = prepare_step_funded(
        &contract,
        &current,
        principal.clone(),
        Transition::Update,
        vec![],
        0,
        12,
        sponsor_point(3),
        |sign, m| {
            witness::update(
                &current,
                1,
                updated.clone(),
                &sign(&key(211)),
                &sign(&operator()),
                m,
            )
        },
    )?;
    advance(
        "update",
        step,
        &oracle,
        &mut current,
        &mut principal,
        &mut rows,
    )?;

    let step = prepare_step_funded(
        &contract,
        &current,
        principal.clone(),
        Transition::Split,
        vec![],
        0,
        13,
        sponsor_point(4),
        |sign, m| {
            witness::carve(
                &current,
                witness::Carve {
                    source_position: 0,
                    claim_position: 4,
                    claim: member(214, 15_000),
                    signature: &sign(&key(210)),
                    preimage: None,
                },
                contract.config().dust_min,
                m,
            )
        },
    )?;
    advance(
        "split",
        step,
        &oracle,
        &mut current,
        &mut principal,
        &mut rows,
    )?;

    let claim = member(215, 10_000);
    let voucher = sign_voucher(
        current
            .leaf(1)
            .unwrap()
            .voucher_message(contract.config().tree_id, &claim),
        &key(221),
    );
    let step = prepare_step_funded(
        &contract,
        &current,
        principal.clone(),
        Transition::Claim,
        vec![],
        0,
        14,
        sponsor_point(5),
        |_, m| {
            witness::carve(
                &current,
                witness::Carve {
                    source_position: 1,
                    claim_position: 5,
                    claim: claim.clone(),
                    signature: &voucher,
                    preimage: None,
                },
                contract.config().dust_min,
                m,
            )
        },
    )?;
    advance(
        "claim",
        step,
        &oracle,
        &mut current,
        &mut principal,
        &mut rows,
    )?;

    let preimage = [42; 32];
    let claim = member(216, 5_000);
    let voucher = sign_voucher(
        current.leaf(0).unwrap().hashlocked_voucher_message(
            contract.config().tree_id,
            &claim,
            sha256::Hash::hash(&preimage).to_byte_array(),
        ),
        &key(210),
    );
    let step = prepare_step_funded(
        &contract,
        &current,
        principal.clone(),
        Transition::ClaimHashlock,
        vec![],
        0,
        15,
        sponsor_point(6),
        |_, m| {
            witness::carve(
                &current,
                witness::Carve {
                    source_position: 0,
                    claim_position: 6,
                    claim: claim.clone(),
                    signature: &voucher,
                    preimage: Some(preimage),
                },
                contract.config().dust_min,
                m,
            )
        },
    )?;
    advance(
        "hashlocked_claim",
        step,
        &oracle,
        &mut current,
        &mut principal,
        &mut rows,
    )?;

    let step = prepare_step_funded(
        &contract,
        &current,
        principal.clone(),
        Transition::Exit { count: 1 },
        vec![payout(current.leaf(2).unwrap())],
        0,
        16,
        sponsor_point(7),
        |sign, m| witness::exit(&current, &[(2, false, sign(&key(212)))], m),
    )?;
    advance(
        "exit",
        step,
        &oracle,
        &mut current,
        &mut principal,
        &mut rows,
    )?;

    let payouts = [3, 4]
        .map(|position| payout(current.leaf(position).unwrap()))
        .to_vec();
    let step = prepare_step_funded(
        &contract,
        &current,
        principal.clone(),
        Transition::Exit { count: 2 },
        payouts,
        0,
        17,
        sponsor_point(8),
        |sign, m| {
            witness::exit(
                &current,
                &[(3, false, sign(&key(213))), (4, false, sign(&key(214)))],
                m,
            )
        },
    )?;
    advance(
        "paired_exit",
        step,
        &oracle,
        &mut current,
        &mut principal,
        &mut rows,
    )?;

    let payouts = [8, 9]
        .map(|position| payout(current.leaf(position).unwrap()))
        .to_vec();
    let step = prepare_step_funded(
        &contract,
        &current,
        principal.clone(),
        Transition::EvictSubtree { height: 2 },
        payouts,
        LEASE,
        18,
        sponsor_point(9),
        |sign, m| witness::evict_subtree(&current, 2, 2, &sign(&operator()), m),
    )?;
    advance(
        "cohort_eviction",
        step,
        &oracle,
        &mut current,
        &mut principal,
        &mut rows,
    )?;

    let step = prepare_step_funded(
        &contract,
        &current,
        principal.clone(),
        Transition::EvictSubtree { height: 1 },
        vec![payout(current.leaf(5).unwrap())],
        LEASE,
        19,
        sponsor_point(10),
        |sign, m| witness::evict_subtree(&current, 1, 2, &sign(&operator()), m),
    )?;
    advance(
        "eviction",
        step,
        &oracle,
        &mut current,
        &mut principal,
        &mut rows,
    )?;

    Ok(serde_json::json!({
        "profile": "LayerTree v0.15.2 dense MID, executed Script in WASM",
        "initial_address": deposit,
        "initial_principal": 120_000,
        "sponsor_address": sponsor_address(),
        "final_principal": current.root().sum,
        "remaining_members": current.leaves().count(),
        "steps": rows,
    }))
}
