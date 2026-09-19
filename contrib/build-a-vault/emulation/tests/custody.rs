//! Complete the public custody routes with real WASM evaluation and signatures.

use bitcoin::bip32::{Xpriv, Xpub};
use bitcoin::hashes::Hash;
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{
    Address, Amount, CompressedPublicKey, Network, OutPoint, PublicKey, ScriptBuf, Transaction,
    TxOut, Txid,
};
use build_a_vault_blocks::{AddressTarget, KeySet, RecoveryRule, RelativeDelay};
use build_a_vault_emulation::{OpVault, OracleRoot, WithdrawalProposal};
use emulator_connect::program::completion::SpendIntent;
use emulator_connect::program::spend_plan::PlanCheck;
use emulator_connect::program::{
    plan_spends, prepare_spend, ProgramCapability, ProgramEvidence, ProgramOracle,
    ProgramSpendPath, SpendAssets, SpendPath,
};
use sapio::contract::{Compiled, Context};
use sapio::template::Template;
use sapio_base::covenant::LoweringPlan;
use sapio_base::effects::EffectPath;
use sapio_base::op_vault::{Recovery, Trigger};
use sapio_base::program::ProgramInstance;
use sapio_psbt::selected::SignatureSlot;
use sapio_psbt::SigningKey;
use sapio_wasm_plugin::client::plugin::Callable;
use std::sync::Arc;

const PRINCIPAL: u64 = 100_000;
const FEE: u64 = 1_000;

fn root(seed: u8) -> Xpriv {
    Xpriv::new_master(Network::Regtest, &[seed; 32]).unwrap()
}

fn keys(seeds: &[u8], threshold: u8) -> KeySet {
    KeySet {
        threshold,
        keys: seeds
            .iter()
            .map(|seed| {
                root(*seed)
                    .to_keypair(&Secp256k1::new())
                    .x_only_public_key()
                    .0
            })
            .collect(),
    }
}

fn destination(seed: u8) -> AddressTarget {
    AddressTarget {
        address: Address::p2tr(
            &Secp256k1::new(),
            keys(&[seed], 1).keys[0],
            None,
            Network::Regtest,
        )
        .into_unchecked(),
    }
}

fn terms(withdrawal_sats: u64) -> OpVault {
    OpVault {
        trigger: keys(&[1, 2, 3], 2),
        recovery: RecoveryRule {
            authorization: keys(&[4, 5], 2),
            destination: destination(6),
        },
        delay: RelativeDelay { blocks: 144 },
        oracle: OracleRoot {
            xpub: Xpub::from_priv(&Secp256k1::new(), &root(90)),
        },
        proposal: WithdrawalProposal {
            destination: destination(7),
            withdrawal_sats,
        },
    }
}

fn context(amount: u64) -> Context {
    Context::new(
        Network::Regtest,
        Amount::from_sat(amount),
        LoweringPlan::Native,
        EffectPath::try_from("custody").unwrap(),
        Arc::new(Default::default()),
        None,
    )
}

fn template<'a>(object: &'a Compiled, first_output: &str) -> &'a Template {
    object
        .suggested_txs
        .values()
        .find(|template| template.funding_constraints.as_ref().unwrap().outputs[0] == first_output)
        .unwrap()
}

fn fund(object: &Compiled, candidate: &Template, parent: Option<(&Transaction, u32)>) -> Psbt {
    let mut psbt = Psbt::from_unsigned_tx(candidate.tx.clone()).unwrap();
    assert_eq!(psbt.inputs.len(), 2);
    psbt.unsigned_tx.input[0].previous_output = parent.map_or(
        OutPoint {
            txid: Txid::from_byte_array([21; 32]),
            vout: 0,
        },
        |(transaction, vout)| OutPoint {
            txid: transaction.compute_txid(),
            vout,
        },
    );
    psbt.inputs[0].witness_utxo = Some(parent.map_or_else(
        || TxOut {
            value: object.required_input_amount,
            script_pubkey: ScriptBuf::from(&object.address),
        },
        |(transaction, vout)| transaction.output[vout as usize].clone(),
    ));
    psbt.unsigned_tx.input[1].previous_output = OutPoint {
        txid: Txid::from_byte_array([22; 32]),
        vout: u32::from(parent.is_some()),
    };
    let sponsor = CompressedPublicKey(root(91).to_keypair(&Secp256k1::new()).public_key());
    psbt.inputs[1].witness_utxo = Some(TxOut {
        value: Amount::from_sat(FEE),
        script_pubkey: Address::p2wpkh(&sponsor, Network::Regtest).script_pubkey(),
    });
    psbt
}

fn prepare(
    object: &Compiled,
    candidate: &Template,
    instance: &ProgramInstance,
    psbt: Psbt,
    native: &[u8],
) -> SpendIntent {
    let requirement = object
        .program_requirements()
        .unwrap()
        .into_iter()
        .find(|requirement| requirement.program.instance() == instance)
        .unwrap();
    let path = match requirement.path {
        ProgramSpendPath::ScriptPath(leaf) => SpendPath::ScriptPath(leaf),
        ProgramSpendPath::KeyPath => panic!("vault predicates must be script paths"),
    };
    let witness = candidate
        .metadata_map_s2s
        .extra
        .get("op_vault_witness")
        .map(|value| serde_json::from_value::<Vec<u8>>(value.clone()).unwrap())
        .unwrap_or_default();
    let assets = SpendAssets {
        schnorr_keys: keys(native, 1).keys.into_iter().collect(),
        programs: vec![ProgramCapability {
            requirement: requirement.clone(),
            codec: "build-a-vault/example-v1".into(),
            evidence_available: true,
            signer_available: true,
        }],
        ..Default::default()
    };
    let evidence = ProgramEvidence {
        requirement,
        codec: "build-a-vault/example-v1".into(),
        witness,
    };
    SpendIntent::from_prepared(prepare_spend(object, path, psbt, 0, &assets, &[evidence]).unwrap())
}

fn complete(object: &Compiled, intent: &SpendIntent, native: &[u8]) -> Transaction {
    let secp = Secp256k1::new();
    let mut current = intent.baseline_psbt().clone();
    assert!(intent.finalize(object, &current, &secp).is_err());
    intent
        .sign_native(
            object,
            &mut current,
            &SigningKey(native.iter().map(|seed| root(*seed)).collect()),
            &secp,
        )
        .unwrap();
    let oracle = ProgramOracle::new(root(90), vec![]).unwrap();
    assert_eq!(intent.requests().len(), 1);
    let signed = oracle.sign(intent.requests()[0].clone()).unwrap();
    intent
        .merge_response(object, &mut current, 0, &signed)
        .unwrap();
    // The sponsor is a separate, ordinary wallet signature. It is never
    // authorized through the covenant oracle or the vault's native key set.
    let sponsor = PublicKey::new(root(91).to_keypair(&secp).public_key());
    assert_eq!(
        SigningKey(vec![root(91)])
            .sign_selected_input_mut(&mut current, &secp, 1, &[SignatureSlot::Ecdsa(sponsor)],)
            .unwrap(),
        1
    );
    let saved: SpendIntent = serde_json::from_slice(&serde_json::to_vec(intent).unwrap()).unwrap();
    let current = Psbt::deserialize(&current.serialize()).unwrap();
    let finalized = saved.finalize(object, &current, &secp).unwrap();
    let transaction = finalized.extract_tx().unwrap();
    assert!(transaction
        .input
        .iter()
        .all(|input| !input.witness.is_empty()));
    assert!(transaction
        .input
        .iter()
        .all(|input| input.script_sig.is_empty()));
    assert_eq!(
        transaction
            .output
            .iter()
            .map(|output| output.value.to_sat())
            .sum::<u64>(),
        object.required_input_amount.to_sat()
    );
    transaction
}

#[test]
fn single_signer_guards_round_trip_and_complete_both_source_routes() {
    let mut terms = terms(60_000);
    terms.trigger = keys(&[1], 1);
    terms.recovery.authorization = keys(&[4], 1);
    let object = terms.call(context(PRINCIPAL)).unwrap();
    let mut input = bitcoin::psbt::Input::default();
    object
        .descriptor
        .as_ref()
        .unwrap()
        .update_psbt_input(&mut input)
        .unwrap();
    assert_eq!(input.tap_scripts.len(), 2);
    for (script, _) in input.tap_scripts.values() {
        let parsed = sapio_base::miniscript::Miniscript::<
            bitcoin::XOnlyPublicKey,
            sapio_base::miniscript::Tap,
        >::decode_consensus(script)
        .unwrap();
        assert_eq!(parsed.encode(), *script);
    }
    let trigger = Trigger::new(terms.delay.blocks, terms.oracle.xpub).unwrap();
    let recovery = Recovery::new(
        &terms
            .recovery
            .destination
            .checked(Network::Regtest)
            .unwrap()
            .script_pubkey(),
        terms.oracle.xpub,
    )
    .unwrap();
    for (route, instance, native) in [
        ("pending", trigger.instance(), &[1][..]),
        ("recovery", recovery.instance(), &[4][..]),
    ] {
        let candidate = template(&object, route);
        let intent = prepare(
            &object,
            candidate,
            instance,
            fund(&object, candidate, None),
            native,
        );
        complete(&object, &intent, native);
    }
}

#[test]
fn trigger_and_delayed_withdrawal_complete_with_a_separate_fee_sponsor() {
    for withdrawal in [PRINCIPAL, 60_000] {
        let terms = terms(withdrawal);
        let object = terms.call(context(PRINCIPAL)).unwrap();
        object.validate().unwrap();
        assert!(object.ctv_to_tx.is_empty());
        let trigger = Trigger::new(terms.delay.blocks, terms.oracle.xpub).unwrap();
        let candidate = template(&object, "pending");
        let intent = prepare(
            &object,
            candidate,
            trigger.instance(),
            fund(&object, candidate, None),
            &[1, 2],
        );
        let transaction = complete(&object, &intent, &[1, 2]);
        assert_eq!(transaction.output[0].value.to_sat(), withdrawal);
        assert_eq!(
            transaction.output.len(),
            if withdrawal == PRINCIPAL { 1 } else { 2 }
        );
        if withdrawal < PRINCIPAL {
            assert_eq!(transaction.output[1].value.to_sat(), PRINCIPAL - withdrawal);
            assert_eq!(
                transaction.output[1].script_pubkey,
                ScriptBuf::from(&object.address)
            );
        }

        let pending = &candidate.outputs[0].contract;
        pending.validate().unwrap();
        let withdrawal = template(pending, "withdrawal");
        assert_eq!(withdrawal.tx.input[0].sequence.to_consensus_u32(), 144);
        let predicate = trigger
            .withdrawal_program(sapio_base::Ctv(withdrawal.hash()))
            .unwrap();
        let mut early = fund(pending, withdrawal, Some((&transaction, 0)));
        early.unsigned_tx.input[0].sequence = bitcoin::Sequence(143);
        let requirement = pending
            .program_requirements()
            .unwrap()
            .into_iter()
            .find(|requirement| requirement.program.instance() == predicate.instance())
            .unwrap();
        let ProgramSpendPath::ScriptPath(leaf) = requirement.path else {
            panic!("expected delayed leaf");
        };
        let report = plan_spends(pending, Some((&early, 0)), &SpendAssets::default()).unwrap();
        assert_eq!(
            report
                .branches
                .iter()
                .find(|branch| branch.path == SpendPath::ScriptPath(leaf))
                .unwrap()
                .transaction_compatible,
            PlanCheck::Unmet
        );
        let intent = prepare(
            pending,
            withdrawal,
            predicate.instance(),
            fund(pending, withdrawal, Some((&transaction, 0))),
            &[],
        );
        let released = complete(pending, &intent, &[]);
        assert_eq!(released.output.len(), 1);
        assert_eq!(
            released.output[0].script_pubkey,
            terms
                .proposal
                .destination
                .checked(Network::Regtest)
                .unwrap()
                .script_pubkey()
        );
        assert_eq!(
            released.input[0].previous_output.txid,
            transaction.compute_txid()
        );
    }
}

#[test]
fn recovery_preserves_principal_before_and_after_triggering() {
    let terms = terms(60_000);
    let object = terms.call(context(PRINCIPAL)).unwrap();
    let recovery_script = terms
        .recovery
        .destination
        .checked(Network::Regtest)
        .unwrap()
        .script_pubkey();
    let predicate = Recovery::new(&recovery_script, terms.oracle.xpub).unwrap();
    let direct = template(&object, "recovery");
    let intent = prepare(
        &object,
        direct,
        predicate.instance(),
        fund(&object, direct, None),
        &[4, 5],
    );
    let recovered = complete(&object, &intent, &[4, 5]);
    assert_eq!(recovered.output[0].value.to_sat(), PRINCIPAL);
    assert_eq!(recovered.output[0].script_pubkey, recovery_script);

    let trigger = Trigger::new(terms.delay.blocks, terms.oracle.xpub).unwrap();
    let candidate = template(&object, "pending");
    let intent = prepare(
        &object,
        candidate,
        trigger.instance(),
        fund(&object, candidate, None),
        &[1, 2],
    );
    let triggered = complete(&object, &intent, &[1, 2]);
    let pending = &candidate.outputs[0].contract;
    let recovery = template(pending, "recovery");
    let intent = prepare(
        pending,
        recovery,
        predicate.instance(),
        fund(pending, recovery, Some((&triggered, 0))),
        &[4, 5],
    );
    let recovered = complete(pending, &intent, &[4, 5]);
    assert_eq!(recovered.output[0].value.to_sat(), 60_000);
    assert_eq!(recovered.output[0].script_pubkey, recovery_script);
    assert_eq!(
        recovered.input[0].previous_output.txid,
        triggered.compute_txid()
    );
    assert_eq!(
        pending
            .program_requirements()
            .unwrap()
            .into_iter()
            .find(|requirement| requirement.program.instance() == predicate.instance())
            .unwrap()
            .path,
        object
            .program_requirements()
            .unwrap()
            .into_iter()
            .find(|requirement| requirement.program.instance() == predicate.instance())
            .unwrap()
            .path
    );
}

#[test]
fn withdrawal_proposals_change_pending_outputs_but_not_the_funding_policy() {
    let initial = terms(60_000);
    let first = initial.call(context(PRINCIPAL)).unwrap();
    let mut changed = initial.clone();
    changed.proposal.destination = destination(8);
    changed.proposal.withdrawal_sats = 70_000;
    let second = changed.call(context(PRINCIPAL)).unwrap();
    assert_eq!(first.address, second.address);
    assert_eq!(first.descriptor, second.descriptor);
    assert_eq!(first.program_policies, second.program_policies);
    assert_ne!(
        template(&first, "pending").tx.output,
        template(&second, "pending").tx.output
    );
    assert_eq!(
        template(&first, "recovery").tx,
        template(&second, "recovery").tx
    );
    // A revaulted remainder reuses these spending terms at its new balance.
    changed.proposal.withdrawal_sats = 20_000;
    assert_eq!(
        first.address,
        changed.call(context(40_000)).unwrap().address
    );
    for amount in [0, PRINCIPAL + 1] {
        let mut invalid = initial.clone();
        invalid.proposal.withdrawal_sats = amount;
        assert!(invalid.call(context(PRINCIPAL)).is_err());
    }
}

#[test]
fn evaluators_enforce_principal_commitments_and_exact_evidence_encodings() {
    let terms = terms(60_000);
    let object = terms.call(context(PRINCIPAL)).unwrap();
    let oracle = ProgramOracle::new(root(90), vec![]).unwrap();
    let recovery = Recovery::new(
        &terms
            .recovery
            .destination
            .checked(Network::Regtest)
            .unwrap()
            .script_pubkey(),
        terms.oracle.xpub,
    )
    .unwrap();
    let candidate = template(&object, "recovery");
    let intent = prepare(
        &object,
        candidate,
        recovery.instance(),
        fund(&object, candidate, None),
        &[4, 5],
    );
    let valid = &intent.requests()[0];
    let mut below_principal = valid.clone();
    below_principal.psbt.0.unsigned_tx.output[0].value -= Amount::ONE_SAT;
    assert!(oracle.sign(below_principal).is_err());
    let mut malformed = valid.clone();
    malformed.witness.pop();
    assert!(oracle.sign(malformed).is_err());
    let mut trailing = valid.clone();
    trailing.witness.push(0);
    assert!(oracle.sign(trailing).is_err());

    let trigger = Trigger::new(terms.delay.blocks, terms.oracle.xpub).unwrap();
    let candidate = template(&object, "pending");
    let intent = prepare(
        &object,
        candidate,
        trigger.instance(),
        fund(&object, candidate, None),
        &[1, 2],
    );
    let mut truncated_proof = intent.requests()[0].clone();
    truncated_proof.witness.pop();
    assert!(oracle.sign(truncated_proof).is_err());
    let mut trailing_proof = intent.requests()[0].clone();
    trailing_proof.witness.push(0);
    assert!(oracle.sign(trailing_proof).is_err());

    let pending = &candidate.outputs[0].contract;
    let withdrawal = template(pending, "withdrawal");
    let predicate = trigger
        .withdrawal_program(sapio_base::Ctv(withdrawal.hash()))
        .unwrap();
    let intent = prepare(
        pending,
        withdrawal,
        predicate.instance(),
        fund(pending, withdrawal, None),
        &[],
    );
    let mut changed_destination = intent.requests()[0].clone();
    changed_destination.psbt.0.unsigned_tx.output[0].script_pubkey = destination(8)
        .checked(Network::Regtest)
        .unwrap()
        .script_pubkey();
    assert!(oracle.sign(changed_destination).is_err());
}
