//! Covenant checks use the actual distributed MID Script guest.

use bitcoin::bip32::Xpub;
use bitcoin::consensus::deserialize;
use bitcoin::hashes::{sha256, Hash};
use bitcoin::hex::FromHex;
use bitcoin::{Amount, ScriptBuf, Sequence, Transaction, TxOut};
use emulator_connect::program::{ProgramError, ProgramSigningRequest, ProgramSpendPath};
use sapio_base::bitcoin_script::encode_stack;
use sapio_base::secp::secp;
use sapio_contrib::contracts::layertree::{
    tree::Node, witness, Config, LayerTree, Leaf, Transition, Tree,
};
use sapio_integration_tests::layertree_example as example;

fn read_stack(request: &ProgramSigningRequest) -> Vec<Vec<u8>> {
    let mut bytes = request.witness.as_slice();
    let read = |bytes: &mut &[u8]| {
        let value = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        *bytes = &bytes[4..];
        value
    };
    let count = read(&mut bytes);
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        let length = read(&mut bytes);
        items.push(bytes[..length].to_vec());
        bytes = &bytes[length..];
    }
    assert!(bytes.is_empty());
    items
}

fn replace_stack(request: &mut ProgramSigningRequest, items: &[Vec<u8>]) {
    request.witness = encode_stack(items).unwrap();
}

fn resign_exit(request: &mut ProgramSigningRequest, signer: u8) {
    let ProgramSpendPath::ScriptPath(leaf) = request.path else {
        panic!("LayerTree script-path fixture")
    };
    let mut items = read_stack(request);
    let slot = items.len() - 4;
    items[slot] = example::sign_transaction(&request.psbt.0, leaf, &example::key(signer)).unwrap();
    replace_stack(request, &items);
}

fn denied(request: ProgramSigningRequest) {
    assert!(matches!(
        example::oracle().unwrap().sign(request),
        Err(ProgramError::Rejected | ProgramError::Evaluation(_))
    ));
}

fn principal(contract: &LayerTree, tree: &Tree, tag: u8) -> example::Coin {
    let compiled = example::compile(contract, tree).unwrap();
    example::Coin {
        outpoint: example::outpoint(tag),
        txout: TxOut {
            value: Amount::from_sat(tree.root().sum),
            script_pubkey: (&compiled.address).into(),
        },
    }
}

fn successor(transaction: &Transaction) -> example::Coin {
    example::Coin {
        outpoint: bitcoin::OutPoint::new(transaction.compute_txid(), 0),
        txout: transaction.output[0].clone(),
    }
}

fn claim_step(
    contract: &LayerTree,
    current: &Tree,
    coin: example::Coin,
    claim: &Leaf,
    voucher: &[u8],
    preimage: Option<[u8; 32]>,
    tag: u8,
) -> example::Step {
    example::prepare_step(
        contract,
        current,
        coin,
        if preimage.is_some() {
            Transition::ClaimHashlock
        } else {
            Transition::Claim
        },
        vec![],
        0,
        tag,
        |_, scripts| {
            witness::carve(
                current,
                witness::Carve {
                    source_position: 0,
                    claim_position: 4,
                    claim: claim.clone(),
                    signature: voucher,
                    preimage,
                },
                contract.config().dust_min,
                scripts,
            )
        },
    )
    .unwrap()
}

#[test]
fn linked_script_transitions_preserve_principal_fee_ownership_and_payouts() {
    let scenario = example::scenario().unwrap();
    assert_eq!(scenario["initial_principal"], 120_000);
    assert_eq!(scenario["final_principal"], 75_000);
    assert_eq!(scenario["remaining_members"], 3);
    let rows = scenario["steps"].as_array().unwrap();
    assert_eq!(rows.len(), 11);
    let mut previous = None;
    for row in rows {
        let bytes = Vec::<u8>::from_hex(row["transaction"].as_str().unwrap()).unwrap();
        let transaction: Transaction = deserialize(&bytes).unwrap();
        assert_eq!(transaction.input.len(), 2);
        assert!(!transaction.input[0].witness.is_empty());
        assert!(!transaction.input[1].witness.is_empty());
        if let Some(previous) = previous {
            assert_eq!(transaction.input[0].previous_output, previous);
        }
        previous = Some(bitcoin::OutPoint::new(transaction.compute_txid(), 0));
        assert_eq!(
            transaction.output[0].value.to_sat(),
            row["new_principal"].as_u64().unwrap()
        );
        let input_total =
            row["old_principal"].as_u64().unwrap() + row["sponsor_sats"].as_u64().unwrap();
        let output_total: u64 = transaction
            .output
            .iter()
            .map(|output| output.value.to_sat())
            .sum();
        assert_eq!(input_total - output_total, example::FEE_SATS);
    }
}

#[test]
fn fresh_owner_signatures_cannot_change_state_principal_or_exact_payout() {
    let step = example::exit_fixture().unwrap();
    let original = step.intent.requests()[0].clone();
    example::oracle().unwrap().sign(original.clone()).unwrap();
    for mutation in 0..6 {
        let mut changed = original.clone();
        match mutation {
            0 => changed.psbt.0.unsigned_tx.output[0].value += Amount::ONE_SAT,
            1 => changed.psbt.0.unsigned_tx.output[0].value -= Amount::ONE_SAT,
            2 => {
                changed.psbt.0.unsigned_tx.output[0].script_pubkey =
                    example::payout(&example::member(213, 100_000)).script_pubkey
            }
            3 => changed.psbt.0.unsigned_tx.output[1].value -= Amount::ONE_SAT,
            4 => {
                changed.psbt.0.unsigned_tx.output[1].script_pubkey =
                    example::payout(&example::member(213, 20_000)).script_pubkey
            }
            _ => {
                changed.psbt.0.inputs[0]
                    .witness_utxo
                    .as_mut()
                    .unwrap()
                    .value += Amount::ONE_SAT
            }
        }
        // Exercise the covenant after owner authorization succeeds on the
        // modified BIP341 transaction, rather than relying on a stale signature.
        resign_exit(&mut changed, 212);
        denied(changed);
    }
    let mut wrong_owner = original;
    resign_exit(&mut wrong_owner, 213);
    denied(wrong_owner);
}

#[test]
fn script_authenticates_membership_and_rejects_ambiguous_field_framing() {
    let step = example::exit_fixture().unwrap();
    let original = step.intent.requests()[0].clone();
    let items = read_stack(&original);
    for slot in [items.len() - 1, items.len() - 2, 9] {
        let mut changed = original.clone();
        let mut corrupted = items.clone();
        assert_eq!(corrupted[slot].len(), 32);
        corrupted[slot][0] ^= 1;
        replace_stack(&mut changed, &corrupted);
        denied(changed);
    }
    let mut changed = original.clone();
    let mut corrupted = items.clone();
    // The outer sibling amount must be a canonical nonnegative Script number.
    corrupted[10] = vec![0];
    replace_stack(&mut changed, &corrupted);
    denied(changed);

    let mut changed = original;
    let mut corrupted = items;
    let leaf = corrupted.len() - 17;
    assert_eq!(corrupted[leaf].len(), 8);
    assert_eq!(corrupted[leaf + 1].len(), 32);
    // Preserve the complete concatenated leaf preimage while shifting one
    // byte between adjacent fields. Script must reject their individual sizes.
    let moved = corrupted[leaf].pop().unwrap();
    corrupted[leaf + 1].insert(0, moved);
    replace_stack(&mut changed, &corrupted);
    denied(changed);
}

#[test]
fn eviction_requires_a_genuine_evictor_signature_and_mature_nonfinal_locktime() {
    let contract = example::contract().unwrap();
    let tree = example::initial_tree().unwrap();
    let step = example::prepare_step(
        &contract,
        &tree,
        principal(&contract, &tree, 1),
        Transition::Exit { count: 1 },
        vec![example::payout(tree.leaf(2).unwrap())],
        example::LEASE,
        2,
        |sign, scripts| witness::exit(&tree, &[(2, true, sign(&example::operator()))], scripts),
    )
    .unwrap();
    let original = step.intent.requests()[0].clone();
    example::oracle().unwrap().sign(original.clone()).unwrap();
    let mut early = original.clone();
    early.psbt.0.unsigned_tx.lock_time =
        bitcoin::absolute::LockTime::from_consensus(example::LEASE - 1);
    resign_exit(&mut early, 203);
    denied(early);
    let mut final_sequence = original.clone();
    final_sequence.psbt.0.unsigned_tx.input[0].sequence = Sequence::MAX;
    resign_exit(&mut final_sequence, 203);
    denied(final_sequence);
    let mut wrong_authority = original;
    resign_exit(&mut wrong_authority, 212);
    denied(wrong_authority);
    // The owner can leave before the eviction lease, without the operator.
    let owner = example::exit_fixture().unwrap();
    example::oracle()
        .unwrap()
        .sign(owner.intent.requests()[0].clone())
        .unwrap();
}

#[test]
fn admission_and_update_require_operator_and_owner_authority_independently() {
    let contract = example::contract().unwrap();
    let tree = example::initial_tree().unwrap();
    let coin = principal(&contract, &tree, 1);
    let joined = example::prepare_step(
        &contract,
        &tree,
        coin.clone(),
        Transition::Join { height: 0 },
        vec![],
        0,
        2,
        |sign, scripts| {
            witness::join(
                &tree,
                0,
                3,
                &[(3, example::member(213, 30_000))],
                &sign(&example::operator()),
                scripts,
            )
        },
    )
    .unwrap();
    let mut joined_request = joined.intent.requests()[0].clone();
    example::oracle()
        .unwrap()
        .sign(joined_request.clone())
        .unwrap();
    let ProgramSpendPath::ScriptPath(join_leaf) = joined_request.path else {
        panic!("script-path join")
    };
    let mut items = read_stack(&joined_request);
    let slot = items.len() - 3;
    items[slot] =
        example::sign_transaction(&joined_request.psbt.0, join_leaf, &example::key(213)).unwrap();
    replace_stack(&mut joined_request, &items);
    denied(joined_request);

    let mut record = tree.leaf(1).unwrap().clone();
    record.owner_key = example::key(221).x_only_public_key().0;
    let updated = example::prepare_step(
        &contract,
        &tree,
        coin,
        Transition::Update,
        vec![],
        0,
        3,
        |sign, scripts| {
            witness::update(
                &tree,
                1,
                record.clone(),
                &sign(&example::key(211)),
                &sign(&example::operator()),
                scripts,
            )
        },
    )
    .unwrap();
    let original = updated.intent.requests()[0].clone();
    example::oracle().unwrap().sign(original.clone()).unwrap();
    let ProgramSpendPath::ScriptPath(leaf) = original.path else {
        panic!("script-path update")
    };
    for bad_slot in [3, 4] {
        let mut changed = original.clone();
        let mut items = read_stack(&changed);
        let slot = items.len() - bad_slot;
        items[slot] = example::sign_transaction(&changed.psbt.0, leaf, &example::key(212)).unwrap();
        replace_stack(&mut changed, &items);
        denied(changed);
    }
    let mut changed = original;
    changed.psbt.0.unsigned_tx.output[0].value += Amount::ONE_SAT;
    let mut items = read_stack(&changed);
    let owner = items.len() - 4;
    let operator = items.len() - 3;
    items[owner] = example::sign_transaction(&changed.psbt.0, leaf, &example::key(211)).unwrap();
    items[operator] =
        example::sign_transaction(&changed.psbt.0, leaf, &example::operator()).unwrap();
    replace_stack(&mut changed, &items);
    denied(changed);
}

#[test]
fn vouchers_cannot_replay_after_sequence_advance_and_hashlocks_bind_exact_preimages() {
    let contract = example::contract().unwrap();
    let tree = example::initial_tree().unwrap();
    let claim = example::member(214, 10_000);
    let voucher = example::sign_voucher(
        tree.leaf(0)
            .unwrap()
            .voucher_message(contract.config().tree_id, &claim),
        &example::key(210),
    );
    let step = claim_step(
        &contract,
        &tree,
        principal(&contract, &tree, 1),
        &claim,
        &voucher,
        None,
        2,
    );
    let claimed = example::finish(&step, &example::oracle().unwrap()).unwrap();
    assert_eq!(step.next.leaf(0).unwrap().seq, 1);
    // Leave the new leaf to make its position and identity reusable. The
    // source generation remains advanced in this linked, accepted state.
    let exit = example::prepare_step(
        &contract,
        &step.next,
        successor(&claimed),
        Transition::Exit { count: 1 },
        vec![example::payout(&claim)],
        0,
        3,
        |sign, scripts| witness::exit(&step.next, &[(4, false, sign(&example::key(214)))], scripts),
    )
    .unwrap();
    let exited = example::finish(&exit, &example::oracle().unwrap()).unwrap();
    let replay = claim_step(
        &contract,
        &exit.next,
        successor(&exited),
        &claim,
        &voucher,
        None,
        4,
    );
    let mut request = replay.intent.requests()[0].clone();
    denied(request.clone());
    let fresh = example::sign_voucher(
        exit.next
            .leaf(0)
            .unwrap()
            .voucher_message(contract.config().tree_id, &claim),
        &example::key(210),
    );
    let mut items = read_stack(&request);
    let slot = items.len() - 3;
    items[slot] = fresh;
    replace_stack(&mut request, &items);
    example::oracle().unwrap().sign(request).unwrap();

    let preimage = [42; 32];
    let voucher = example::sign_voucher(
        tree.leaf(0).unwrap().hashlocked_voucher_message(
            contract.config().tree_id,
            &claim,
            sha256::Hash::hash(&preimage).to_byte_array(),
        ),
        &example::key(210),
    );
    let step = claim_step(
        &contract,
        &tree,
        principal(&contract, &tree, 5),
        &claim,
        &voucher,
        Some(preimage),
        6,
    );
    let mut request = step.intent.requests()[0].clone();
    example::oracle().unwrap().sign(request.clone()).unwrap();
    let mut items = read_stack(&request);
    let slot = items.len() - 3;
    assert_eq!(items[slot], preimage);
    items[slot][0] ^= 1;
    replace_stack(&mut request, &items);
    denied(request);
}

#[test]
fn final_member_exit_closes_the_pool_and_depth_sixteen_remains_executable() {
    for depth in [1, 16] {
        let contract = LayerTree::new(
            Config {
                depth,
                ..example::contract().unwrap().config().clone()
            },
            Xpub::from_priv(secp(), &example::oracle_key()),
        )
        .unwrap();
        let mut tree = Tree::new(depth).unwrap();
        tree.set(0, Some(example::member(210, 30_000))).unwrap();
        if depth == 16 {
            tree.set(65_535, Some(example::member(211, 20_000)))
                .unwrap();
        }
        let step = example::prepare_step(
            &contract,
            &tree,
            principal(&contract, &tree, 1),
            Transition::Exit { count: 1 },
            vec![example::payout(tree.leaf(0).unwrap())],
            0,
            2,
            |sign, scripts| witness::exit(&tree, &[(0, false, sign(&example::key(210)))], scripts),
        )
        .unwrap();
        let transaction = example::finish(&step, &example::oracle().unwrap()).unwrap();
        assert_eq!(
            transaction.output[1],
            example::payout(tree.leaf(0).unwrap())
        );
        if depth == 1 {
            assert_eq!(step.next.root(), Node::empty(depth));
            assert_eq!(transaction.output[0].value, Amount::ZERO);
            assert_eq!(
                transaction.output[0].script_pubkey,
                ScriptBuf::from_bytes(vec![0x6a])
            );
        } else {
            assert_eq!(step.next.root().sum, 20_000);
            assert_eq!(transaction.output[0].value.to_sat(), 20_000);
        }
    }
}
