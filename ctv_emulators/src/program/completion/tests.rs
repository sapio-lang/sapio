use super::*;
use crate::program::spend_plan::{
    plan_spends, prepare_spend, ProgramCapability, ProgramEvidence, SpendAssets,
};
use crate::program::{ProgramOracle, WasmEvaluator};
use bitcoin::bip32::{Xpriv, Xpub};
use bitcoin::hashes::{sha256, Hash};
use bitcoin::{
    absolute, Amount, FeeRate, Network, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
    XOnlyPublicKey,
};
use sapio::contract::{Compilable, Compiled, Context};
use sapio::template::{OutputAmount, Template};
use sapio_base::miniscript::Descriptor;
use sapio_base::policy::ScriptPolicy;
use sapio_base::program::{EmulatedProgram, ProgramInstance};
use sapio_base::{Clause, LoweringPlan};
use sapio_psbt::selected::StackElement;
use std::str::FromStr;
use std::sync::Arc;

fn root(seed: u8) -> Xpriv {
    Xpriv::new_master(Network::Regtest, &[seed; 32]).unwrap()
}

fn key(seed: u8) -> XOnlyPublicKey {
    Xpub::from_priv(&Secp256k1::new(), &root(seed))
        .public_key
        .x_only_public_key()
        .0
}

struct Source {
    policy: ScriptPolicy,
    rate: Option<FeeRate>,
}

#[sapio::contract]
impl Source {
    #[policy]
    fn spend(&self) -> ScriptPolicy {
        self.policy.clone()
    }

    #[action(suggested, default, guarded_by(Self::spend))]
    fn payment(&self, ctx: Context) -> Result<Template, sapio::template::PlanError> {
        let recipient = Compiled::from_descriptor(
            Descriptor::<XOnlyPublicKey>::from_str(&format!("tr({})", key(80))).unwrap(),
            Amount::ZERO,
        );
        let mut plan = ctx.template_plan();
        plan.output(
            "payment",
            OutputAmount::Exact(Amount::from_sat(900)),
            &recipient,
        )?;
        plan.reserve_fees(Amount::from_sat(100));
        if let Some(rate) = self.rate {
            plan.require_feerate(rate);
        }
        plan.finish()
    }
}

fn compile(source: Source) -> Object {
    source
        .compile(Context::new(
            Network::Regtest,
            Amount::from_sat(1_000),
            LoweringPlan::Native,
            "completion".try_into().unwrap(),
            Arc::new(Default::default()),
            None,
        ))
        .unwrap()
}

fn funded(object: &Object) -> Psbt {
    let transaction = object
        .suggested_txs
        .values()
        .next()
        .map(|template| template.tx.clone())
        .unwrap_or_else(|| Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![TxIn {
                sequence: Sequence::ZERO,
                ..TxIn::default()
            }],
            output: vec![TxOut {
                value: Amount::from_sat(900),
                script_pubkey: ScriptBuf::new(),
            }],
        });
    let mut psbt = Psbt::from_unsigned_tx(transaction).unwrap();
    psbt.inputs[0].witness_utxo = Some(TxOut {
        value: Amount::from_sat(1_000),
        script_pubkey: ScriptBuf::from(&object.address),
    });
    psbt
}

fn evaluator() -> WasmEvaluator {
    WasmEvaluator::new(wat::parse_str(include_str!("../pay_at_least_test.wat")).unwrap()).unwrap()
}

fn program(seed: u8, minimum: u64) -> EmulatedProgram {
    EmulatedProgram::new(
        ProgramInstance::new(
            evaluator().id(),
            b"pay-at-least".to_vec(),
            minimum.to_le_bytes().to_vec(),
        )
        .unwrap(),
        Xpub::from_priv(&Secp256k1::new(), &root(seed)),
    )
    .unwrap()
}

fn prepare(object: &Object, native: &[XOnlyPublicKey]) -> SpendIntent {
    let psbt = funded(object);
    let requirements = object.program_requirements().unwrap();
    let assets = SpendAssets {
        schnorr_keys: native.iter().copied().collect(),
        programs: requirements
            .iter()
            .map(|requirement| ProgramCapability {
                requirement: requirement.clone(),
                codec: "test/output-index".into(),
                evidence_available: true,
                signer_available: true,
            })
            .collect(),
        ..Default::default()
    };
    let report = plan_spends(object, Some((&psbt, 0)), &assets).unwrap();
    let selected = report
        .branches
        .iter()
        .find(|branch| branch.status == BranchStatus::Planned)
        .unwrap();
    let evidence: Vec<_> = requirements
        .into_iter()
        .filter(|requirement| {
            selected.requirements.iter().any(|selected| {
                matches!(selected, SpendRequirement::Program { requirement: chosen, .. } if chosen == requirement)
            })
        })
        .map(|requirement| ProgramEvidence {
            requirement,
            codec: "test/output-index".into(),
            witness: 0_u32.to_le_bytes().to_vec(),
        })
        .collect();
    SpendIntent::from_prepared(
        prepare_spend(object, selected.path, psbt, 0, &assets, &evidence).unwrap(),
    )
}

fn mixed(rate: Option<FeeRate>) -> (Object, SpendIntent) {
    let object = compile(Source {
        policy: ScriptPolicy::And(vec![
            Clause::Key(key(40)).into(),
            program(41, 100).into(),
            program(42, 200).into(),
        ]),
        rate,
    });
    let intent = prepare(&object, &[key(40)]);
    (object, intent)
}

fn responses(object: &Object, intent: &SpendIntent) -> Vec<Psbt> {
    let requirements = object.program_requirements().unwrap();
    intent
        .requests()
        .iter()
        .map(|request| {
            let requirement = requirements
                .iter()
                .find(|requirement| {
                    requirement.program.instance() == &request.instance
                        && requirement.path == request.path
                })
                .unwrap();
            let seed = [41, 42]
                .into_iter()
                .find(|seed| {
                    Xpub::from_priv(&Secp256k1::new(), &root(*seed)) == *requirement.program.root()
                })
                .unwrap();
            ProgramOracle::new(root(seed), vec![evaluator()])
                .unwrap()
                .sign(request.clone())
                .unwrap()
        })
        .collect()
}

#[test]
fn two_program_responses_merge_out_of_order_after_reload_with_a_native_signature() {
    let secp = Secp256k1::new();
    let (object, intent) = mixed(None);
    assert_eq!(intent.requests().len(), 2);
    let returned = responses(&object, &intent);
    let mut current = intent.baseline_psbt().clone();
    intent
        .sign_native(&object, &mut current, &SigningKey(vec![root(40)]), &secp)
        .unwrap();
    assert_eq!(current.inputs[0].tap_script_sigs.len(), 1);
    assert!(current.inputs[0].tap_key_sig.is_none());
    intent
        .merge_response(&object, &mut current, 1, &returned[1])
        .unwrap();
    assert!(matches!(
        intent.finalize(&object, &current, &secp),
        Err(SpendCompletionError::Incomplete(_))
    ));
    let object_bytes = serde_json::to_vec(&object).unwrap();
    let intent_bytes = serde_json::to_vec(&intent).unwrap();
    let current_bytes = current.serialize();
    drop((object, intent, current));

    let restored: Object = serde_json::from_slice(&object_bytes).unwrap();
    let intent: SpendIntent = serde_json::from_slice(&intent_bytes).unwrap();
    let mut current = Psbt::deserialize(&current_bytes).unwrap();
    let first = current.inputs[0].tap_script_sigs.clone();
    intent
        .merge_response(&restored, &mut current, 0, &returned[0])
        .unwrap();
    assert!(contains(&first, &current.inputs[0].tap_script_sigs));
    let complete = current.clone();
    intent
        .merge_response(&restored, &mut current, 1, &returned[1])
        .unwrap();
    assert_eq!(current, complete);
    assert_eq!(
        intent.status(&restored, &current).unwrap().status,
        BranchStatus::Planned
    );
    let finalized = intent.finalize(&restored, &current, &secp).unwrap();
    assert_eq!(current, complete);
    assert_eq!(
        finalized.inputs[0]
            .final_script_witness
            .as_ref()
            .unwrap()
            .len(),
        5
    );
    let template = restored.suggested_txs.values().next().unwrap();
    assert_eq!(
        template
            .check_funded_psbt(&finalized)
            .unwrap()
            .actual_fee_sats,
        Some(100)
    );
    assert_eq!(
        finalized.extract_tx().unwrap().output[0].value,
        Amount::from_sat(900)
    );
}

#[test]
fn responses_are_bound_to_their_original_request_and_only_contribute_one_slot() {
    let (object, intent) = mixed(None);
    let returned = responses(&object, &intent);
    let mut current = intent.baseline_psbt().clone();
    let before = current.clone();
    assert!(intent
        .merge_response(&object, &mut current, 0, &returned[1])
        .is_err());
    assert_eq!(current, before);
    let mut changed = returned[0].clone();
    changed.unsigned_tx.output[0].value += Amount::ONE_SAT;
    assert!(intent
        .merge_response(&object, &mut current, 0, &changed)
        .is_err());
    assert_eq!(current, before);
    let mut extra = returned[0].clone();
    extra.inputs[0]
        .tap_script_sigs
        .extend(returned[1].inputs[0].tap_script_sigs.clone());
    assert!(intent
        .merge_response(&object, &mut current, 0, &extra)
        .is_err());
    assert_eq!(current, before);
    assert!(matches!(
        intent.merge_response(&object, &mut current, 2, &returned[0]),
        Err(SpendCompletionError::RequestIndex(2))
    ));
    assert_eq!(current, before);
}

#[test]
fn fixed_fields_and_saved_request_identities_are_checked_before_mutation() {
    let (object, intent) = mixed(None);
    let returned = responses(&object, &intent);
    let mut changed = intent.baseline_psbt().clone();
    changed.inputs[0].witness_utxo.as_mut().unwrap().value += Amount::ONE_SAT;
    let before = changed.clone();
    assert!(intent
        .merge_response(&object, &mut changed, 0, &returned[0])
        .is_err());
    assert_eq!(changed, before);
    let mut altered = intent.clone();
    altered.program_requests.swap(0, 1);
    assert!(altered.status(&object, altered.baseline_psbt()).is_err());
    let mut altered = intent.clone();
    altered.program_requests[0].psbt.0.unsigned_tx.output[0].value -= Amount::ONE_SAT;
    assert!(altered.status(&object, altered.baseline_psbt()).is_err());
    let mut altered = intent.clone();
    altered.recipe.witness.push(StackElement::Literal(vec![1]));
    assert!(altered.status(&object, altered.baseline_psbt()).is_err());
    let mut altered = intent.clone();
    altered.program_requests.clear();
    assert!(altered.status(&object, altered.baseline_psbt()).is_err());
}

#[test]
fn native_signing_does_not_take_another_available_branch_or_replace_original_assets() {
    let object = Compiled::from_descriptor(
        Descriptor::<XOnlyPublicKey>::from_str(&format!(
            "tr({},or_i(pk({}),pk({})))",
            key(40),
            key(41),
            key(42)
        ))
        .unwrap(),
        Amount::from_sat(1_000),
    );
    let intent = prepare(&object, &[key(42)]);
    let secp = Secp256k1::new();
    let mut current = intent.baseline_psbt().clone();
    intent
        .sign_native(
            &object,
            &mut current,
            &SigningKey(vec![root(40), root(41), root(42)]),
            &secp,
        )
        .unwrap();
    assert!(current.inputs[0].tap_key_sig.is_none());
    assert_eq!(current.inputs[0].tap_script_sigs.len(), 1);
    assert!(current.inputs[0]
        .tap_script_sigs
        .keys()
        .all(|(key, _)| *key == self::key(42)));
    let original = current.clone();
    intent
        .sign_native(&object, &mut current, &SigningKey(vec![root(42)]), &secp)
        .unwrap();
    assert_eq!(current, original);
    let finalized = intent.finalize(&object, &current, &secp).unwrap();
    assert_eq!(
        finalized.inputs[0]
            .final_script_witness
            .as_ref()
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn final_weight_constraints_are_checked_after_signature_verification() {
    let (object, intent) = mixed(Some(FeeRate::from_sat_per_vb(1).unwrap()));
    let returned = responses(&object, &intent);
    let secp = Secp256k1::new();
    let mut current = intent.baseline_psbt().clone();
    intent
        .sign_native(&object, &mut current, &SigningKey(vec![root(40)]), &secp)
        .unwrap();
    for (index, response) in returned.iter().enumerate() {
        intent
            .merge_response(&object, &mut current, index, response)
            .unwrap();
    }
    assert_eq!(
        intent.status(&object, &current).unwrap().status,
        BranchStatus::Planned
    );
    let before = current.clone();
    assert!(matches!(
        intent.finalize(&object, &current, &secp),
        Err(SpendCompletionError::Funding(_))
    ));
    assert_eq!(current, before);
}

#[test]
fn key_path_requests_keep_local_proofs_out_of_the_signer_request() {
    let object = compile(Source {
        policy: program(41, 100).into(),
        rate: None,
    });
    let intent = prepare(&object, &[]);
    assert_eq!(intent.path, SpendPath::KeyPath);
    assert!(intent.baseline_psbt().inputs[0].tap_internal_key.is_some());
    let request = &intent.requests()[0];
    assert!(request.psbt.0.inputs[0].tap_internal_key.is_none());
    assert!(request.psbt.0.inputs[0].tap_scripts.is_empty());
    assert!(request.psbt.0.inputs[0].tap_key_origins.is_empty());
    let response = ProgramOracle::new(root(41), vec![evaluator()])
        .unwrap()
        .sign(request.clone())
        .unwrap();
    let restored: SpendIntent =
        serde_json::from_slice(&serde_json::to_vec(&intent).unwrap()).unwrap();
    let mut current = restored.baseline_psbt().clone();
    restored
        .merge_response(&object, &mut current, 0, &response)
        .unwrap();
    assert!(current.inputs[0].tap_internal_key.is_some());
    assert_eq!(
        restored
            .finalize(&object, &current, &Secp256k1::new())
            .unwrap()
            .inputs[0]
            .final_script_witness
            .as_ref()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn saved_native_assets_cannot_be_removed_and_preimage_replacements_fail() {
    let (object, intent) = mixed(None);
    let mut saved = intent.clone();
    let preimage = vec![9; 32];
    let hash = sha256::Hash::hash(&preimage);
    saved.baseline.0.inputs[0]
        .sha256_preimages
        .insert(hash, preimage);
    // This is a native asset in the initial common PSBT, hence also in each
    // original program request. Restore still rederives their full identities.
    for request in &mut saved.program_requests {
        request.psbt.0.inputs[0].sha256_preimages =
            saved.baseline.0.inputs[0].sha256_preimages.clone();
    }
    let mut current = saved.baseline_psbt().clone();
    saved.status(&object, &current).unwrap();
    current.inputs[0].sha256_preimages.insert(hash, vec![8; 32]);
    assert!(matches!(
        saved.status(&object, &current),
        Err(SpendCompletionError::ChangedPSBT(_))
    ));
}

#[test]
fn uncatalogued_spends_must_cover_outputs_before_completion_succeeds() {
    let object = Compiled::from_descriptor(
        Descriptor::<XOnlyPublicKey>::from_str(&format!("tr({})", key(40))).unwrap(),
        Amount::from_sat(1_000),
    );
    let mut psbt = funded(&object);
    psbt.unsigned_tx.output[0].value = Amount::from_sat(1_001);
    let assets = SpendAssets {
        schnorr_keys: [key(40)].into(),
        ..Default::default()
    };
    let prepared = prepare_spend(
        &object,
        SpendPath::KeyPath,
        psbt,
        0,
        &assets,
        &[] as &[ProgramEvidence],
    )
    .unwrap();
    let intent = SpendIntent::from_prepared(prepared);
    let secp = Secp256k1::new();
    let mut current = intent.baseline_psbt().clone();
    intent
        .sign_native(&object, &mut current, &SigningKey(vec![root(40)]), &secp)
        .unwrap();
    let before = current.clone();
    assert!(matches!(
        intent.finalize(&object, &current, &secp),
        Err(SpendCompletionError::Balance(_))
    ));
    assert_eq!(current, before);
}
