use super::*;
use bitcoin::bip32::{Xpriv, Xpub};
use bitcoin::secp256k1::{Keypair, Message, Secp256k1, SecretKey};
use bitcoin::sighash::{Prevouts, SighashCache};
use bitcoin::{Amount, Network, OutPoint, Sequence, TapSighashType, Transaction, TxIn, TxOut};
use sapio::contract::abi::object::{RawTaproot, SupportedDescriptors};
use sapio::contract::{Compilable, Compiled, Context, Contract};
use sapio::{declare, guard};
use sapio_base::policy::ScriptPolicy;
use sapio_base::program::{EmulatedProgram, ProgramInstance};
use sapio_base::util::CTVHash;
use sapio_base::{Clause, LoweringPlan};
use std::sync::Arc;

fn keypair(seed: u8) -> Keypair {
    Keypair::from_secret_key(
        &Secp256k1::new(),
        &SecretKey::from_slice(&[seed; 32]).unwrap(),
    )
}
fn key(seed: u8) -> XOnlyPublicKey {
    keypair(seed).x_only_public_key().0
}
fn native(expression: &str) -> Object {
    Compiled::from_descriptor(
        Descriptor::<XOnlyPublicKey>::from_str(expression).unwrap(),
        Amount::from_sat(1_000),
    )
}
fn funded(object: &Object) -> Psbt {
    let transaction = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            sequence: Sequence::ZERO,
            ..TxIn::default()
        }],
        output: vec![TxOut {
            value: Amount::from_sat(900),
            script_pubkey: ScriptBuf::new(),
        }],
    };
    let mut psbt = Psbt::from_unsigned_tx(transaction).unwrap();
    psbt.inputs[0].witness_utxo = Some(TxOut {
        value: Amount::from_sat(1_000),
        script_pubkey: ScriptBuf::from(&object.address),
    });
    psbt
}
fn leaf(report: &SpendReport) -> &BranchPlan {
    report
        .branches
        .iter()
        .find(|plan| matches!(plan.path, SpendPath::ScriptPath(_)))
        .unwrap()
}
fn signatures(plan: &BranchPlan) -> BTreeSet<XOnlyPublicKey> {
    plan.requirements
        .iter()
        .filter_map(|requirement| match requirement {
            SpendRequirement::SchnorrSignature { key, .. } => Some(*key),
            _ => None,
        })
        .collect()
}

#[test]
fn selects_a_complete_available_native_alternative_without_other_keys() {
    let object = native(&format!(
        "tr({},and_v(v:pk({}),or_i(pk({}),pk({}))))",
        key(1),
        key(2),
        key(3),
        key(4)
    ));
    let mut assets = SpendAssets::default();
    assets.schnorr_keys.extend([key(2), key(4)]);
    let report = plan_spends(&object, None, &assets).unwrap();
    assert_eq!(report.branches.len(), 2);
    assert_eq!(report.branches[0].status, BranchStatus::MissingAssets);
    assert_eq!(leaf(&report).status, BranchStatus::Planned);
    assert_eq!(signatures(leaf(&report)), [key(2), key(4)].into());
    assert_eq!(leaf(&report).transaction_compatible, PlanCheck::Unknown);
    assert_eq!(leaf(&report).witness_template.len(), 5); // two signatures, selector, script, proof
    assets.schnorr_keys.remove(&key(2));
    assert_eq!(
        leaf(&plan_spends(&object, None, &assets).unwrap()).status,
        BranchStatus::MissingAssets
    );
    let decoded: SpendReport =
        serde_json::from_slice(&serde_json::to_vec(&report).unwrap()).unwrap();
    assert_eq!(decoded, report);
    let _ = schemars::schema_for!(SpendReport);
}

#[test]
fn native_hashed_key_descriptors_keep_the_original_public_key() {
    let object = native(&format!("tr({},pkh({}))", key(1), key(2)));
    let assets = SpendAssets {
        schnorr_keys: [key(2)].into(),
        ..Default::default()
    };
    let report = plan_spends(&object, None, &assets).unwrap();
    assert_eq!(leaf(&report).status, BranchStatus::Planned);
    assert_eq!(signatures(leaf(&report)), [key(2)].into());
    let prepared = prepare_native(&object, funded(&object), &assets);
    let SpendPath::ScriptPath(hash) = prepared.plan.path else {
        unreachable!()
    };
    assert_eq!(
        prepared.recipe.witness[1],
        StackElement::Literal(key(2).serialize().to_vec())
    );
    let mut psbt = prepared.psbt;
    sign_leaf(&mut psbt, hash, 2);
    sapio_psbt::selected::finalize_selected(
        &mut psbt,
        &Secp256k1::verification_only(),
        &[(0, prepared.recipe)].into(),
    )
    .unwrap();
}

#[test]
fn lock_checks_separate_fixed_transaction_fields_from_unknown_chain_maturity() {
    let object = native(&format!(
        "tr({},and_v(v:pk({}),and_v(v:after(100),older(6))))",
        key(1),
        key(2)
    ));
    let assets = SpendAssets {
        schnorr_keys: [key(2)].into(),
        ..Default::default()
    };
    let mut psbt = funded(&object);
    let report = plan_spends(&object, Some((&psbt, 0)), &assets).unwrap();
    assert_eq!(leaf(&report).status, BranchStatus::IncompatibleTransaction);
    assert!(leaf(&report)
        .requirements
        .contains(&SpendRequirement::AbsoluteTimelock {
            value: 100,
            transaction: PlanCheck::Unmet,
            chain_maturity: PlanCheck::Unknown
        }));
    psbt.unsigned_tx.lock_time = absolute::LockTime::from_consensus(100);
    psbt.unsigned_tx.input[0].sequence = Sequence::from_height(6);
    let report = plan_spends(&object, Some((&psbt, 0)), &assets).unwrap();
    assert_eq!(leaf(&report).status, BranchStatus::Planned);
    assert!(leaf(&report)
        .requirements
        .contains(&SpendRequirement::RelativeTimelock {
            value: 6,
            transaction: PlanCheck::Met,
            chain_maturity: PlanCheck::Unknown
        }));
    psbt.unsigned_tx.version = bitcoin::transaction::Version::ONE;
    assert_eq!(
        leaf(&plan_spends(&object, Some((&psbt, 0)), &assets).unwrap()).status,
        BranchStatus::IncompatibleTransaction
    );
    psbt.unsigned_tx.version = bitcoin::transaction::Version::TWO;
    psbt.unsigned_tx.input[0].sequence = Sequence::MAX;
    assert_eq!(
        leaf(&plan_spends(&object, Some((&psbt, 0)), &assets).unwrap()).status,
        BranchStatus::IncompatibleTransaction
    );
}

#[test]
fn preimages_are_checked_and_pending_signature_is_not_a_verified_signature() {
    let preimage = [29; 32];
    let hash = sha256::Hash::hash(&preimage);
    let object = native(&format!(
        "tr({},and_v(v:pk({}),sha256({hash})))",
        key(1),
        key(2)
    ));
    let mut psbt = funded(&object);
    let mut assets = SpendAssets {
        schnorr_keys: [key(2)].into(),
        ..Default::default()
    };
    psbt.inputs[0].sha256_preimages.insert(hash, vec![0; 32]);
    let report = plan_spends(&object, Some((&psbt, 0)), &assets).unwrap();
    assert!(leaf(&report)
        .requirements
        .contains(&SpendRequirement::Preimage {
            hash: HashRequirement::Sha256(hash),
            availability: Availability::Missing
        }));
    psbt.inputs[0]
        .sha256_preimages
        .insert(hash, preimage.to_vec());
    let report = plan_spends(&object, Some((&psbt, 0)), &assets).unwrap();
    assert!(leaf(&report)
        .requirements
        .contains(&SpendRequirement::Preimage {
            hash: HashRequirement::Sha256(hash),
            availability: Availability::PresentVerified
        }));
    let SpendPath::ScriptPath(hash) = leaf(&report).path else {
        unreachable!()
    };
    psbt.inputs[0].tap_script_sigs.insert(
        (key(2), hash),
        bitcoin::taproot::Signature {
            signature: Secp256k1::new()
                .sign_schnorr_no_aux_rand(&Message::from_digest([0; 32]), &keypair(2)),
            sighash_type: TapSighashType::Default,
        },
    );
    assets.schnorr_keys.clear();
    let report = plan_spends(&object, Some((&psbt, 0)), &assets).unwrap();
    assert!(leaf(&report)
        .requirements
        .contains(&SpendRequirement::SchnorrSignature {
            key: key(2),
            availability: Availability::PresentUnverified
        }));
}

struct Source(ScriptPolicy);
impl Source {
    #[guard(policy, cached)]
    fn spend(self) -> ScriptPolicy {
        self.0.clone()
    }
}
impl Contract for Source {
    declare! {finish, Self::spend}
}

fn program(seed: u8) -> EmulatedProgram {
    let root = Xpriv::new_master(Network::Regtest, &[seed; 32]).unwrap();
    EmulatedProgram::new(
        ProgramInstance::wasm_v2(vec![0, 97, 115, 109, 1, 0, 0, 0], vec![]).unwrap(),
        Xpub::from_priv(&Secp256k1::new(), &root),
    )
    .unwrap()
}
fn program_object() -> Object {
    Source(ScriptPolicy::And(vec![
        Clause::Key(key(2)).into(),
        Clause::Older(miniscript::RelLockTime::from_height(6)).into(),
        program(4).into(),
        program(5).into(),
    ]))
    .compile(Context::new(
        Network::Regtest,
        Amount::from_sat(1_000),
        LoweringPlan::Native,
        "spend_plan".try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    ))
    .unwrap()
}

#[test]
fn two_programs_and_native_guards_share_one_complete_plan_and_explicit_requests() {
    let object = program_object();
    let mut psbt = funded(&object);
    psbt.unsigned_tx.input[0].sequence = Sequence::from_height(6);
    let requirements = object.program_requirements().unwrap();
    assert_eq!(requirements.len(), 2);
    let mut assets = SpendAssets::default();
    for requirement in &requirements {
        assets
            .schnorr_keys
            .insert(requirement.program.derive_public_key().unwrap());
    }
    let report = plan_spends(&object, Some((&psbt, 0)), &assets).unwrap();
    assert_eq!(leaf(&report).status, BranchStatus::MissingAssets);
    assert_eq!(
        leaf(&report)
            .requirements
            .iter()
            .filter(|item| matches!(
                item,
                SpendRequirement::Program {
                    signature: Availability::Missing,
                    ..
                }
            ))
            .count(),
        2
    );
    let path = leaf(&report).path;
    assets.programs = requirements
        .iter()
        .map(|requirement| ProgramCapability {
            requirement: requirement.clone(),
            codec: "test/v1".into(),
            evidence_available: false,
            signer_available: true,
        })
        .collect();
    assert!(matches!(
        prepare_spend(
            &object,
            path,
            psbt.clone(),
            0,
            &assets,
            &Vec::<ProgramEvidence>::new()
        ),
        Err(SpendPlanError::MissingEvidence(_))
    ));
    let evidence: Vec<_> = requirements
        .iter()
        .enumerate()
        .map(|(index, requirement)| ProgramEvidence {
            requirement: requirement.clone(),
            codec: "test/v1".into(),
            witness: vec![index as u8],
        })
        .collect();
    let prepared = prepare_spend(&object, path, psbt.clone(), 0, &assets, &evidence).unwrap();
    assert_eq!(prepared.program_requests.len(), 2);
    assert_eq!(prepared.input_index, 0);
    let SpendPath::ScriptPath(selected_leaf) = path else {
        unreachable!()
    };
    assert_eq!(
        prepared.native_signatures,
        vec![SignatureSlot::Schnorr {
            key: key(2),
            leaf: Some(selected_leaf)
        }]
    );
    let (resumed, native) =
        validate_spend_selection(&object, path, &prepared.psbt, 0, &prepared.recipe).unwrap();
    assert_eq!(native, prepared.native_signatures);
    assert_eq!(resumed.status, BranchStatus::MissingAssets);
    assert_eq!(
        resumed
            .requirements
            .iter()
            .filter(|item| matches!(
                item,
                SpendRequirement::Program {
                    signature: Availability::Missing,
                    evidence: Availability::Missing,
                    signer_available: false,
                    ..
                }
            ))
            .count(),
        2
    );
    assert_eq!(prepared.plan.status, BranchStatus::MissingAssets); // native key remains pending
    assert_eq!(signatures(&prepared.plan), [key(2)].into());
    for request in &prepared.program_requests {
        assert_eq!(request.psbt.0, prepared.psbt);
        assert_eq!(request.psbt.0.unsigned_tx, psbt.unsigned_tx);
        assert!(!request.psbt.0.inputs[0].tap_scripts.is_empty());
        assert!(request.psbt.0.inputs[0].tap_script_sigs.is_empty());
        assert_eq!(
            SpendPath::ScriptPath(match request.path {
                ProgramSpendPath::ScriptPath(hash) => hash,
                _ => panic!("script-path request expected"),
            }),
            path
        );
    }
    let mut wrong_codec = evidence.clone();
    wrong_codec[0].codec = "other/v1".into();
    assert!(matches!(
        prepare_spend(&object, path, psbt.clone(), 0, &assets, &wrong_codec),
        Err(SpendPlanError::InvalidEvidence)
    ));
    let mut wrong_root = evidence.clone();
    wrong_root[0].requirement.program = program(9);
    assert!(matches!(
        prepare_spend(&object, path, psbt.clone(), 0, &assets, &wrong_root),
        Err(SpendPlanError::InvalidEvidence)
    ));
    psbt.unsigned_tx.input[0].sequence = Sequence::ZERO;
    assert!(matches!(
        prepare_spend(&object, path, psbt, 0, &assets, &evidence),
        Err(SpendPlanError::IncompatibleTransaction)
    ));
}

fn prepare_native(object: &Object, psbt: Psbt, assets: &SpendAssets) -> PreparedSpend {
    let report = plan_spends(object, Some((&psbt, 0)), assets).unwrap();
    let path = if report
        .branches
        .iter()
        .any(|plan| matches!(plan.path, SpendPath::ScriptPath(_)))
    {
        leaf(&report).path
    } else {
        report.branches[0].path
    };
    prepare_spend(object, path, psbt, 0, assets, &[] as &[ProgramEvidence]).unwrap()
}

fn sign_leaf(psbt: &mut Psbt, leaf: TapLeafHash, seed: u8) {
    let prevout = psbt.inputs[0].witness_utxo.clone().unwrap();
    let hash = SighashCache::new(&psbt.unsigned_tx)
        .taproot_script_spend_signature_hash(
            0,
            &Prevouts::All(&[prevout]),
            leaf,
            TapSighashType::Default,
        )
        .unwrap();
    psbt.inputs[0].tap_script_sigs.insert(
        (key(seed), leaf),
        bitcoin::taproot::Signature {
            signature: Secp256k1::new().sign_schnorr_no_aux_rand(
                &Message::from_digest(hash.to_byte_array()),
                &keypair(seed),
            ),
            sighash_type: TapSighashType::Default,
        },
    );
}

#[test]
fn selected_inner_alternative_survives_restart_and_additional_signatures() {
    let object = native(&format!(
        "tr({},or_i(pk({}),pk({})))",
        key(1),
        key(2),
        key(3)
    ));
    let assets = SpendAssets {
        schnorr_keys: [key(2)].into(),
        ..Default::default()
    };
    let prepared = prepare_native(&object, funded(&object), &assets);
    let recipe: SatisfactionRecipe =
        serde_json::from_slice(&serde_json::to_vec(&prepared.recipe).unwrap()).unwrap();
    let path = prepared.plan.path;
    let SpendPath::ScriptPath(hash) = path else {
        unreachable!()
    };
    let mut psbt = prepared.psbt;
    let (missing, native) = validate_spend_selection(&object, path, &psbt, 0, &recipe).unwrap();
    assert_eq!(missing.status, BranchStatus::MissingAssets);
    assert_eq!(
        native,
        vec![SignatureSlot::Schnorr {
            key: key(2),
            leaf: Some(hash)
        }]
    );
    sign_leaf(&mut psbt, hash, 3);
    let (still_missing, _) = validate_spend_selection(&object, path, &psbt, 0, &recipe).unwrap();
    assert_eq!(still_missing.status, BranchStatus::MissingAssets);
    assert_eq!(signatures(&still_missing), [key(2)].into());
    sign_leaf(&mut psbt, hash, 2);
    let (selected, _) = validate_spend_selection(&object, path, &psbt, 0, &recipe).unwrap();
    assert_eq!(selected.status, BranchStatus::Planned);
    assert_eq!(
        selected.satisfaction_weight_upper_bound,
        prepared.plan.satisfaction_weight_upper_bound
    );
    sapio_psbt::selected::finalize_selected(
        &mut psbt,
        &Secp256k1::verification_only(),
        &[(0, recipe)].into(),
    )
    .unwrap();
    let witness = psbt.inputs[0]
        .final_script_witness
        .as_ref()
        .unwrap()
        .to_vec();
    assert_eq!(witness[1], vec![1]); // the selected left OR branch, though the right is now available
    assert_eq!(witness[0].len(), 64);
}

#[test]
fn equal_key_nested_choices_and_threshold_selection_are_canonical() {
    for expression in [
        format!("or_i(pk({}),or_i(pk({}),pk({})))", key(2), key(2), key(3)),
        format!("thresh(2,pk({}),s:pk({}),s:pk({}))", key(2), key(3), key(4)),
    ] {
        // The typed descriptor parser excludes repeated public keys, whereas
        // checked raw Taproot leaves can still contain these native scripts.
        let script = Miniscript::<XOnlyPublicKey, Tap>::from_str_insane(&expression)
            .unwrap()
            .encode();
        let raw = RawTaproot::from_scripts(key(1), vec![script]).unwrap();
        let mut object = Compiled::from_script(
            raw.script_pubkey(),
            Amount::from_sat(1_000),
            Network::Regtest,
        )
        .unwrap();
        object.descriptor = Some(SupportedDescriptors::Taproot(raw));
        let assets = SpendAssets {
            schnorr_keys: [key(2), key(4)].into(),
            ..Default::default()
        };
        let prepared = prepare_native(&object, funded(&object), &assets);
        let path = prepared.plan.path;
        let SpendPath::ScriptPath(hash) = path else {
            unreachable!()
        };
        let mut psbt = prepared.psbt;
        for seed in [2, 3, 4] {
            sign_leaf(&mut psbt, hash, seed);
        }
        let (selected, native) =
            validate_spend_selection(&object, path, &psbt, 0, &prepared.recipe).unwrap();
        assert_eq!(native, prepared.native_signatures, "{expression}");
        assert_eq!(
            signatures(&selected),
            signatures(&prepared.plan),
            "{expression}"
        );
        sapio_psbt::selected::finalize_selected(
            &mut psbt,
            &Secp256k1::verification_only(),
            &[(0, prepared.recipe)].into(),
        )
        .unwrap();
    }
}

#[test]
fn imported_recipe_bytes_and_slots_are_rederived_from_the_artifact() {
    let object = native(&format!(
        "tr({},or_i(pk({}),pk({})))",
        key(1),
        key(2),
        key(3)
    ));
    let prepared = prepare_native(
        &object,
        funded(&object),
        &SpendAssets {
            schnorr_keys: [key(2)].into(),
            ..Default::default()
        },
    );
    let path = prepared.plan.path;
    let mut wrong_selector = prepared.recipe.clone();
    wrong_selector.witness[1] = StackElement::Literal(vec![]);
    let mut wrong_proof = prepared.recipe.clone();
    let StackElement::Literal(proof) = wrong_proof.witness.last_mut().unwrap() else {
        unreachable!()
    };
    proof[0] ^= 1;
    let mut literal_signature = prepared.recipe.clone();
    literal_signature.witness[0] = StackElement::Literal(vec![0; 64]);
    let mut extra_item = prepared.recipe.clone();
    extra_item.script_sig.push(StackElement::Literal(vec![]));
    for wrong in [wrong_selector, wrong_proof, literal_signature, extra_item] {
        assert!(validate_spend_selection(&object, path, &prepared.psbt, 0, &wrong).is_err());
    }
}

#[test]
fn resumed_selection_rechecks_native_locks_preimages_and_annex() {
    let preimage = [7; 32];
    let hash = sha256::Hash::hash(&preimage);
    let object = native(&format!(
        "tr({},and_v(v:pk({}),and_v(v:sha256({hash}),and_v(v:after(100),older(6)))))",
        key(1),
        key(2)
    ));
    let mut psbt = funded(&object);
    psbt.unsigned_tx.lock_time = absolute::LockTime::from_consensus(100);
    psbt.unsigned_tx.input[0].sequence = Sequence::from_height(6);
    sapio_psbt::annex::set(&mut psbt.inputs[0], Some(vec![0x50, 1])).unwrap();
    let assets = SpendAssets {
        schnorr_keys: [key(2)].into(),
        preimages: [HashRequirement::Sha256(hash)].into(),
        ..Default::default()
    };
    let prepared = prepare_native(&object, psbt, &assets);
    let path = prepared.plan.path;
    let mut psbt = prepared.psbt;
    let (missing, _) = validate_spend_selection(&object, path, &psbt, 0, &prepared.recipe).unwrap();
    assert!(missing.requirements.contains(&SpendRequirement::Preimage {
        hash: HashRequirement::Sha256(hash),
        availability: Availability::Missing
    }));
    psbt.inputs[0]
        .sha256_preimages
        .insert(hash, preimage.to_vec());
    let (supplied, _) =
        validate_spend_selection(&object, path, &psbt, 0, &prepared.recipe).unwrap();
    assert!(supplied.requirements.contains(&SpendRequirement::Preimage {
        hash: HashRequirement::Sha256(hash),
        availability: Availability::PresentVerified
    }));
    let mut wrong_lock = psbt.clone();
    wrong_lock.unsigned_tx.input[0].sequence = Sequence::ZERO;
    assert!(matches!(
        validate_spend_selection(&object, path, &wrong_lock, 0, &prepared.recipe),
        Err(SpendPlanError::IncompatibleTransaction)
    ));
    sapio_psbt::annex::set(&mut psbt.inputs[0], Some(vec![0x50, 2])).unwrap();
    assert!(matches!(
        validate_spend_selection(&object, path, &psbt, 0, &prepared.recipe),
        Err(SpendPlanError::InvalidSelection)
    ));
}

#[test]
fn descriptor_recipes_package_witness_scripts_and_nested_redeem_programs() {
    let public = PublicKey::new(keypair(2).public_key());
    let script = Miniscript::<PublicKey, miniscript::Segwitv0>::from_str(&format!("pk({public})"))
        .unwrap()
        .encode();
    for (expression, witness_script, script_sig) in [
        (format!("wsh(pk({public}))"), Some(script.clone()), None),
        (
            format!("sh(wsh(pk({public})))"),
            Some(script.clone()),
            Some(script.to_p2wsh()),
        ),
        (
            format!("sh(wpkh({public}))"),
            None,
            Some(ScriptBuf::new_p2wpkh(&public.wpubkey_hash().unwrap())),
        ),
        (format!("sh(pk({public}))"), None, Some(script.clone())),
    ] {
        let descriptor = Descriptor::<PublicKey>::from_str(&expression).unwrap();
        let object = Compiled::from_descriptor(descriptor, Amount::from_sat(1_000));
        let prepared = prepare_native(
            &object,
            funded(&object),
            &SpendAssets {
                ecdsa_keys: [public].into(),
                ..Default::default()
            },
        );
        let (_, native) = validate_spend_selection(
            &object,
            SpendPath::Descriptor,
            &prepared.psbt,
            0,
            &prepared.recipe,
        )
        .unwrap();
        assert_eq!(native, vec![SignatureSlot::Ecdsa(public)]);
        if let Some(script) = witness_script {
            assert_eq!(
                prepared.recipe.witness.last(),
                Some(&StackElement::Literal(script.into_bytes()))
            );
        }
        if let Some(script) = script_sig {
            assert_eq!(
                prepared.recipe.script_sig.last(),
                Some(&StackElement::Literal(script.into_bytes()))
            );
        }
    }
}

#[test]
fn raw_scripts_stay_unknown_and_preparation_cannot_silently_claim_satisfaction() {
    let raw =
        RawTaproot::from_scripts(key(1), vec![ScriptBuf::from(vec![0x51, 0x6b, 0x6c])]).unwrap();
    let mut object =
        Compiled::from_script(raw.script_pubkey(), Amount::ZERO, Network::Regtest).unwrap();
    object.descriptor = Some(SupportedDescriptors::Taproot(raw));
    let report = plan_spends(&object, None, &SpendAssets::default()).unwrap();
    let unsupported = leaf(&report);
    assert_eq!(unsupported.status, BranchStatus::Unsupported);
    assert_eq!(unsupported.transaction_compatible, PlanCheck::Unknown);
    assert_eq!(unsupported.satisfaction_weight_upper_bound, None);
    assert!(matches!(
        prepare_spend(
            &object,
            unsupported.path,
            funded(&object),
            0,
            &SpendAssets::default(),
            &Vec::<ProgramEvidence>::new()
        ),
        Err(SpendPlanError::UnsupportedBranch)
    ));
}

#[test]
fn malformed_funding_is_rejected_and_missing_prevouts_remain_visible() {
    let object = native(&format!("tr({})", key(1)));
    let mut psbt = funded(&object);
    psbt.inputs[0].witness_utxo = None;
    let report = plan_spends(&object, Some((&psbt, 0)), &SpendAssets::default()).unwrap();
    assert_eq!(report.missing_prevouts, vec![0]);
    assert_eq!(report.funding, PlanCheck::Unknown);
    assert!(matches!(
        prepare_spend(
            &object,
            SpendPath::KeyPath,
            psbt.clone(),
            0,
            &SpendAssets::default(),
            &Vec::<ProgramEvidence>::new()
        ),
        Err(SpendPlanError::MissingPrevouts(_))
    ));
    psbt.inputs[0].non_witness_utxo = Some(psbt.unsigned_tx.clone());
    assert!(matches!(
        plan_spends(&object, Some((&psbt, 0)), &SpendAssets::default()),
        Err(SpendPlanError::Funding(_))
    ));
    let mut psbt = funded(&object);
    psbt.inputs[0].witness_utxo.as_mut().unwrap().script_pubkey = ScriptBuf::new();
    assert!(matches!(
        plan_spends(&object, Some((&psbt, 0)), &SpendAssets::default()),
        Err(SpendPlanError::Artifact(
            ArtifactProgramError::PrevoutMismatch
        ))
    ));
    let mut psbt = funded(&object);
    psbt.inputs.push(Input::default());
    assert!(matches!(
        plan_spends(&object, Some((&psbt, 0)), &SpendAssets::default()),
        Err(SpendPlanError::Artifact(ArtifactProgramError::Program(
            ProgramError::Psbt(_)
        )))
    ));
}

#[test]
fn witness_bound_is_distinct_from_an_observed_valid_final_witness() {
    let object = native(&format!("tr({},pk({}))", key(1), key(2)));
    let mut psbt = funded(&object);
    let assets = SpendAssets {
        schnorr_keys: [key(2)].into(),
        ..Default::default()
    };
    let report = plan_spends(&object, Some((&psbt, 0)), &assets).unwrap();
    let plan = leaf(&report);
    let estimate = plan.witness_bytes_upper_bound.unwrap();
    assert_eq!(report.observed_final_witness_bytes, None);
    let SpendPath::ScriptPath(leaf_hash) = plan.path else {
        unreachable!()
    };
    object
        .descriptor
        .as_ref()
        .unwrap()
        .update_psbt_input(&mut psbt.inputs[0])
        .unwrap();
    let prevout = psbt.inputs[0].witness_utxo.clone().unwrap();
    let hash = SighashCache::new(&psbt.unsigned_tx)
        .taproot_script_spend_signature_hash(
            0,
            &Prevouts::All(&[prevout]),
            leaf_hash,
            TapSighashType::Default,
        )
        .unwrap();
    psbt.inputs[0].tap_script_sigs.insert(
        (key(2), leaf_hash),
        bitcoin::taproot::Signature {
            signature: Secp256k1::new()
                .sign_schnorr_no_aux_rand(&Message::from_digest(hash.to_byte_array()), &keypair(2)),
            sighash_type: TapSighashType::Default,
        },
    );
    let finalized = sapio_psbt::finalize::finalize(psbt, &Secp256k1::verification_only()).unwrap();
    let actual = finalized.inputs[0]
        .final_script_witness
        .as_ref()
        .unwrap()
        .size() as u64;
    assert_eq!(estimate, actual + 1); // 65-byte reserve versus 64-byte DEFAULT signature
    let report = plan_spends(&object, Some((&finalized, 0)), &assets).unwrap();
    assert_eq!(report.observed_final_witness_bytes, Some(actual));
    assert!(
        leaf(&report)
            .satisfaction_weight_upper_bound
            .unwrap()
            .to_wu()
            >= actual + 4
    );
}

#[test]
fn native_wsh_uses_miniscript_planning_and_reports_pending_ecdsa_signature() {
    let public = PublicKey::new(keypair(2).public_key());
    let descriptor = Descriptor::<PublicKey>::from_str(&format!("wsh(pk({public}))")).unwrap();
    let object = Compiled::from_descriptor(descriptor, Amount::from_sat(1_000));
    let assets = SpendAssets {
        ecdsa_keys: [public].into(),
        ..Default::default()
    };
    let report = plan_spends(&object, None, &assets).unwrap();
    let plan = &report.branches[0];
    assert_eq!(plan.path, SpendPath::Descriptor);
    assert_eq!(plan.status, BranchStatus::Planned);
    assert!(plan
        .requirements
        .contains(&SpendRequirement::EcdsaSignature {
            key: public,
            availability: Availability::CanProvide
        }));
    // Stack count, maximum DER+sighash signature, and the actual witness script.
    assert_eq!(plan.witness_bytes_upper_bound, Some(1 + 74 + 36));
    assert_eq!(plan.satisfaction_weight_upper_bound.unwrap().to_wu(), 115);
    assert_eq!(plan.witness_template.len(), 2);
}

#[test]
fn incompatible_native_ctv_conjunction_cannot_claim_a_complete_plan() {
    let first = sha256::Hash::from_byte_array([1; 32]);
    let second = sha256::Hash::from_byte_array([2; 32]);
    let object = native(&format!(
        "tr({},and_v(txtmpl({first}),and_v(txtmpl({second}),pk({}))))",
        key(1),
        key(2)
    ));
    let assets = SpendAssets {
        schnorr_keys: [key(1), key(2)].into(),
        ..Default::default()
    };
    let psbt = funded(&object);
    for funding in [None, Some((&psbt, 0))] {
        let report = plan_spends(&object, funding, &assets).unwrap();
        assert_eq!(report.branches[0].status, BranchStatus::Planned); // key path unaffected
        assert_eq!(leaf(&report).status, BranchStatus::Unsupported);
        assert_eq!(leaf(&report).satisfaction_weight_upper_bound, None);
        assert_eq!(leaf(&report).transaction_compatible, PlanCheck::Unknown);
    }
    let path = leaf(&plan_spends(&object, None, &assets).unwrap()).path;
    assert!(matches!(
        prepare_spend(&object, path, psbt, 0, &assets, &[] as &[ProgramEvidence]),
        Err(SpendPlanError::UnsupportedBranch)
    ));
}

#[test]
fn native_ctv_descriptors_retain_the_selected_commitment() {
    let first = sha256::Hash::from_byte_array([1; 32]);
    let public = PublicKey::new(keypair(2).public_key());
    let assets = SpendAssets {
        ecdsa_keys: [public].into(),
        ..Default::default()
    };
    for expression in [
        format!("wsh(and_v(txtmpl({first}),pk({public})))"),
        format!("sh(wsh(and_v(txtmpl({first}),pk({public}))))"),
        format!("sh(and_v(txtmpl({first}),pk({public})))"),
    ] {
        let descriptor = Descriptor::<PublicKey>::from_str(&expression).unwrap();
        let object = Compiled::from_descriptor(descriptor, Amount::from_sat(1_000));
        let report = plan_spends(&object, None, &assets).unwrap();
        let plan = &report.branches[0];
        assert_eq!(plan.status, BranchStatus::Planned, "{expression}");
        assert!(plan
            .requirements
            .contains(&SpendRequirement::NativeTemplateHash {
                hash: first,
                transaction: PlanCheck::Unknown,
            }));
        assert!(plan.satisfaction_weight_upper_bound.is_some());
    }
}

#[test]
fn native_ctv_fixed_hash_passes_find_a_compatible_outer_conjunction() {
    let first = sha256::Hash::from_byte_array([1; 32]);
    let second = sha256::Hash::from_byte_array([2; 32]);
    let object = native(&format!(
        "tr({},and_v(v:or_i(and_v(txtmpl({first}),pk({})),and_v(txtmpl({second}),pk({}))),t:txtmpl({first})))",
        key(1), key(2), key(3)
    ));
    let assets = SpendAssets {
        schnorr_keys: [key(2), key(3)].into(),
        ..Default::default()
    };
    let report = plan_spends(&object, None, &assets).unwrap();
    assert_eq!(report.branches.len(), 2); // one completion per path, not one per hash
    assert_eq!(leaf(&report).status, BranchStatus::Planned);
    assert_eq!(signatures(leaf(&report)), [key(2)].into());
    assert!(leaf(&report)
        .requirements
        .contains(&SpendRequirement::NativeTemplateHash {
            hash: first,
            transaction: PlanCheck::Unknown,
        }));
}

#[test]
fn native_ctv_checks_funded_fields_before_preparing() {
    let transaction = funded(&native(&format!("tr({})", key(1)))).unsigned_tx;
    let hash = transaction.get_ctv_hash(0);
    let object = native(&format!(
        "tr({},and_v(txtmpl({hash}),pk({})))",
        key(1),
        key(2)
    ));
    let assets = SpendAssets {
        schnorr_keys: [key(2)].into(),
        ..Default::default()
    };
    let mut psbt = funded(&object);
    let report = plan_spends(&object, Some((&psbt, 0)), &assets).unwrap();
    let path = leaf(&report).path;
    assert_eq!(leaf(&report).status, BranchStatus::Planned);
    assert!(leaf(&report)
        .requirements
        .contains(&SpendRequirement::NativeTemplateHash {
            hash,
            transaction: PlanCheck::Met,
        }));
    let prepared = prepare_spend(
        &object,
        path,
        psbt.clone(),
        0,
        &assets,
        &[] as &[ProgramEvidence],
    )
    .unwrap();
    validate_spend_selection(&object, path, &prepared.psbt, 0, &prepared.recipe).unwrap();
    psbt.unsigned_tx.output[0].value = Amount::from_sat(899);
    assert!(matches!(
        validate_spend_selection(&object, path, &psbt, 0, &prepared.recipe),
        Err(SpendPlanError::IncompatibleTransaction)
    ));
    let report = plan_spends(&object, Some((&psbt, 0)), &assets).unwrap();
    assert_eq!(leaf(&report).status, BranchStatus::IncompatibleTransaction);
    assert!(leaf(&report)
        .requirements
        .contains(&SpendRequirement::NativeTemplateHash {
            hash,
            transaction: PlanCheck::Unmet,
        }));
    assert!(matches!(
        prepare_spend(&object, path, psbt, 0, &assets, &[] as &[ProgramEvidence]),
        Err(SpendPlanError::IncompatibleTransaction)
    ));
}

#[test]
fn native_ctv_includes_other_inputs_finalized_script_sigs() {
    let mut psbt = funded(&native(&format!("tr({})", key(1))));
    psbt.unsigned_tx.input.push(TxIn {
        previous_output: OutPoint::new(bitcoin::Txid::from_byte_array([2; 32]), 0),
        ..TxIn::default()
    });
    psbt.inputs.push(bitcoin::psbt::Input {
        witness_utxo: Some(TxOut {
            value: Amount::from_sat(1_000),
            script_pubkey: ScriptBuf::new(),
        }),
        final_script_sig: Some(ScriptBuf::from(vec![0x51])),
        ..Default::default()
    });
    let mut final_transaction = psbt.unsigned_tx.clone();
    final_transaction.input[1].script_sig = psbt.inputs[1].final_script_sig.clone().unwrap();
    let hash = final_transaction.get_ctv_hash(0);
    assert_ne!(hash, psbt.unsigned_tx.get_ctv_hash(0));
    let object = native(&format!(
        "tr({},and_v(txtmpl({hash}),pk({})))",
        key(1),
        key(2)
    ));
    psbt.inputs[0].witness_utxo.as_mut().unwrap().script_pubkey = ScriptBuf::from(&object.address);
    let assets = SpendAssets {
        schnorr_keys: [key(2)].into(),
        ..Default::default()
    };
    let report = plan_spends(&object, Some((&psbt, 0)), &assets).unwrap();
    assert_eq!(leaf(&report).status, BranchStatus::Planned);
    assert!(leaf(&report)
        .requirements
        .contains(&SpendRequirement::NativeTemplateHash {
            hash,
            transaction: PlanCheck::Met,
        }));
    psbt.inputs[1].final_script_sig = None;
    assert_eq!(
        leaf(&plan_spends(&object, Some((&psbt, 0)), &assets).unwrap()).status,
        BranchStatus::IncompatibleTransaction
    );
}

#[test]
fn native_ctv_search_budget_is_shared_across_all_leaves() {
    fn alternatives(branches: &[String]) -> String {
        if branches.len() == 1 {
            return branches[0].clone();
        }
        let (left, right) = branches.split_at(branches.len() / 2);
        format!("or_i({},{})", alternatives(left), alternatives(right))
    }
    let scripts = (0u16..128)
        .map(|leaf| {
            let branches: Vec<_> = (0u8..8)
                .map(|branch| {
                    let hash = sha256::Hash::hash(&[leaf as u8, (leaf >> 8) as u8, branch]);
                    format!("and_v(txtmpl({hash}),pk({}))", key(branch + 2))
                })
                .collect();
            Miniscript::<XOnlyPublicKey, Tap>::from_str(&alternatives(&branches))
                .unwrap()
                .encode()
        })
        .collect();
    let raw = RawTaproot::from_scripts(key(1), scripts).unwrap();
    let mut object = Compiled::from_script(
        raw.script_pubkey(),
        Amount::from_sat(1_000),
        Network::Regtest,
    )
    .unwrap();
    object.descriptor = Some(SupportedDescriptors::Taproot(raw));
    let assets = SpendAssets::default();
    let report = plan_spends(&object, None, &assets).unwrap();
    assert!(report
        .branches
        .iter()
        .any(|plan| matches!(plan.path, SpendPath::ScriptPath(_))
            && plan.status == BranchStatus::MissingAssets));
    let exhausted: Vec<_> = report
        .branches
        .iter()
        .filter(|plan| plan.policy == WORK_LIMIT)
        .collect();
    assert!(!exhausted.is_empty());
    for plan in exhausted {
        assert_eq!(plan.status, BranchStatus::Unsupported);
        assert_eq!(plan.satisfaction_weight_upper_bound, None);
    }
    assert_eq!(report, plan_spends(&object, None, &assets).unwrap());
}

#[test]
fn selected_path_validation_is_not_starved_by_unrelated_leaves() {
    fn alternatives(branches: &[String]) -> String {
        if branches.len() == 1 {
            return branches[0].clone();
        }
        let (left, right) = branches.split_at(branches.len() / 2);
        format!("or_i({},{})", alternatives(left), alternatives(right))
    }
    let ctv = funded(&native(&format!("tr({})", key(1))))
        .unsigned_tx
        .get_ctv_hash(0);
    let target = Miniscript::<XOnlyPublicKey, Tap>::from_str(&format!(
        "and_v(txtmpl({ctv}),pk({}))",
        key(2)
    ))
    .unwrap()
    .encode();
    let target_hash = TapLeafHash::from_script(&target, LeafVersion::TapScript);
    let keys: Vec<_> = (2..10).map(key).collect();
    let mut scripts: Vec<_> = (0u16..u16::MAX)
        .map(|leaf| {
            let branches: Vec<_> = keys
                .iter()
                .enumerate()
                .map(|(index, key)| {
                    let hash = sha256::Hash::hash(&[leaf as u8, (leaf >> 8) as u8, index as u8]);
                    format!("and_v(txtmpl({hash}),pk({key}))")
                })
                .collect();
            Miniscript::<XOnlyPublicKey, Tap>::from_str(&alternatives(&branches))
                .unwrap()
                .encode()
        })
        .filter(|script| TapLeafHash::from_script(script, LeafVersion::TapScript) < target_hash)
        .take(160)
        .collect();
    assert_eq!(scripts.len(), 160);
    scripts.push(target);
    let raw = RawTaproot::from_scripts(key(1), scripts).unwrap();
    let mut object = Compiled::from_script(
        raw.script_pubkey(),
        Amount::from_sat(1_000),
        Network::Regtest,
    )
    .unwrap();
    object.descriptor = Some(SupportedDescriptors::Taproot(raw));
    let psbt = funded(&object);
    let assets = SpendAssets {
        schnorr_keys: [key(2)].into(),
        ..Default::default()
    };
    let report = plan_spends(&object, Some((&psbt, 0)), &assets).unwrap();
    let path = SpendPath::ScriptPath(target_hash);
    let target = report
        .branches
        .iter()
        .find(|plan| plan.path == path)
        .unwrap();
    assert_eq!(target.status, BranchStatus::Unsupported);
    assert_eq!(target.policy, WORK_LIMIT);
    let prepared =
        prepare_spend(&object, path, psbt, 0, &assets, &[] as &[ProgramEvidence]).unwrap();
    let (selected, native) =
        validate_spend_selection(&object, path, &prepared.psbt, 0, &prepared.recipe).unwrap();
    assert_eq!(selected.status, BranchStatus::MissingAssets);
    assert_eq!(
        native,
        vec![SignatureSlot::Schnorr {
            key: key(2),
            leaf: Some(target_hash)
        }]
    );
}

#[test]
fn descriptor_weights_include_wrappers_and_legacy_compact_size_boundaries() {
    let public = [2, 3, 4].map(|seed| PublicKey::new(keypair(seed).public_key()));
    let assets = SpendAssets {
        ecdsa_keys: public.into(),
        ..Default::default()
    };
    for (expression, witness_bytes, satisfaction_weight, stack_items) in [
        (format!("pkh({})", public[0]), 0, 436, 2),
        (format!("wpkh({})", public[0]), 109, 113, 2),
        (format!("sh(wpkh({}))", public[0]), 109, 205, 2),
        (format!("sh(wsh(pk({})))", public[0]), 111, 255, 2),
        (
            format!("sh(multi(2,{},{},{}))", public[0], public[1], public[2]),
            0,
            1036, // 256-byte scriptSig requires a three-byte CompactSize prefix.
            4,
        ),
    ] {
        let descriptor = Descriptor::<PublicKey>::from_str(&expression).unwrap();
        let object = Compiled::from_descriptor(descriptor, Amount::from_sat(1_000));
        let report = plan_spends(&object, None, &assets).unwrap();
        let plan = &report.branches[0];
        assert_eq!(plan.status, BranchStatus::Planned, "{expression}");
        assert_eq!(
            plan.witness_bytes_upper_bound,
            Some(witness_bytes),
            "{expression}"
        );
        assert_eq!(
            plan.satisfaction_weight_upper_bound.unwrap().to_wu(),
            satisfaction_weight,
            "{expression}"
        );
        assert_eq!(plan.witness_template.len(), stack_items, "{expression}");
    }
}

#[test]
fn native_only_preparation_checks_catalogued_fee_constraints() {
    let mut object = native(&format!("tr({})", key(1)));
    let recipient = Compiled::from_descriptor(
        Descriptor::<XOnlyPublicKey>::from_str(&format!("tr({})", key(2))).unwrap(),
        Amount::ZERO,
    );
    let ctx = Context::new(
        Network::Regtest,
        Amount::from_sat(1_000),
        LoweringPlan::Native,
        "fee_plan".try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    );
    let mut plan = ctx.template_plan();
    plan.output(
        "payment",
        sapio::template::OutputAmount::Exact(Amount::from_sat(900)),
        &recipient,
    )
    .unwrap();
    plan.reserve_fees(Amount::from_sat(100));
    let template = plan.finish().unwrap();
    let mut psbt = Psbt::from_unsigned_tx(template.tx.clone()).unwrap();
    psbt.inputs[0].witness_utxo = Some(TxOut {
        value: Amount::from_sat(1_000),
        script_pubkey: ScriptBuf::from(&object.address),
    });
    object.suggested_txs.insert(template.ctv, template);
    let assets = SpendAssets {
        schnorr_keys: [key(1)].into(),
        ..Default::default()
    };
    let report = plan_spends(&object, Some((&psbt, 0)), &assets).unwrap();
    assert_eq!(report.template_funding.unwrap().actual_fee_sats, Some(100));
    psbt.inputs[0].witness_utxo.as_mut().unwrap().value = Amount::from_sat(1_100);
    assert!(matches!(
        prepare_spend(
            &object,
            SpendPath::KeyPath,
            psbt,
            0,
            &assets,
            &Vec::<ProgramEvidence>::new()
        ),
        Err(SpendPlanError::Artifact(ArtifactProgramError::Funding(_)))
    ));
}
