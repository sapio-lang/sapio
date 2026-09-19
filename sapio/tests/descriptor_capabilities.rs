use bitcoin::bip32::{Xpriv, Xpub};
use bitcoin::psbt::Input;
use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
use bitcoin::{Amount, Network, ScriptBuf, XOnlyPublicKey};
use sapio::contract::object::{ArtifactErrorKind, Object, RawTaproot, SupportedDescriptors};
use sapio::contract::{Compilable, Context, Contract};
use sapio::{declare, then};
use sapio_base::miniscript::{Descriptor, Miniscript, Tap};
use sapio_base::{Clause, LoweringPlan};
use std::str::FromStr;
use std::sync::Arc;

struct Payment;

impl Payment {
    #[then]
    fn pay(self, ctx: Context) {
        ctx.template()
            .add_output(Amount::from_sat(1_000), &key(2), None)?
            .into()
    }
}

impl Contract for Payment {
    declare! {actions, Self::pay}
}

fn key(byte: u8) -> XOnlyPublicKey {
    Keypair::from_secret_key(
        &Secp256k1::new(),
        &SecretKey::from_slice(&[byte; 32]).unwrap(),
    )
    .x_only_public_key()
    .0
}

fn payment(lowering: LoweringPlan) -> Object {
    Payment
        .compile(Context::new(
            Network::Regtest,
            Amount::from_sat(1_000),
            lowering,
            "descriptor_capabilities".try_into().unwrap(),
            Arc::new(Default::default()),
            None,
        ))
        .unwrap()
}

fn spending_data(object: &Object) -> (XOnlyPublicKey, Vec<ScriptBuf>) {
    let mut input = Input::default();
    object
        .descriptor
        .as_ref()
        .unwrap()
        .update_psbt_input(&mut input)
        .unwrap();
    (
        input.tap_internal_key.unwrap(),
        input
            .tap_scripts
            .into_values()
            .map(|(script, _)| script)
            .collect(),
    )
}

fn replace_tree(object: &mut Object, key: XOnlyPublicKey, scripts: Vec<ScriptBuf>) {
    let tree = RawTaproot::from_scripts(key, scripts).unwrap();
    object.address =
        bitcoin::Address::p2tr_tweaked(tree.spend_info().output_key(), Network::Regtest).into();
    object.descriptor = Some(tree.into());
}

fn rejects(object: Object) {
    let object: Object = serde_json::from_value(serde_json::to_value(object).unwrap()).unwrap();
    assert!(matches!(
        object.validate().unwrap_err().kind,
        ArtifactErrorKind::InvalidSpendingPolicy(_)
    ));
    assert!(object.explain().is_err());
}

#[test]
fn spliced_covenant_claims_cannot_hide_attacker_internal_keys_or_extra_leaves() {
    let honest = payment(LoweringPlan::Native);
    honest.validate().unwrap();
    let (nums, leaves) = spending_data(&honest);
    let attacker_script = Clause::Key(key(3)).compile::<Tap>().unwrap().encode();

    // The report's exploit: honest claims with an attacker key and pk leaf.
    let mut evil = honest.clone();
    replace_tree(&mut evil, key(3), vec![attacker_script.clone()]);
    rejects(evil);
    // Retaining every honest covenant leaf cannot make a new key path safe.
    let mut evil = honest.clone();
    replace_tree(&mut evil, key(3), leaves.clone());
    rejects(evil);
    // A NUMS key cannot rescue a missing covenant or an added bypass leaf.
    let mut evil = honest.clone();
    replace_tree(&mut evil, nums, vec![attacker_script.clone()]);
    rejects(evil);
    for additional in [attacker_script, ScriptBuf::from(vec![0x51])] {
        let mut evil = honest.clone();
        let mut scripts = leaves.clone();
        scripts.push(additional);
        replace_tree(&mut evil, nums, scripts);
        rejects(evil);
    }
}

#[test]
fn merely_optional_covenants_and_missing_descriptors_fail_closed() {
    let honest = payment(LoweringPlan::Native);
    let hash = honest.ctv_to_tx.keys().next().unwrap();
    let (nums, _) = spending_data(&honest);
    let optional = Miniscript::<XOnlyPublicKey, Tap>::from_str(&format!(
        "or_i(and_v(txtmpl({hash}),1),pk({}))",
        key(3)
    ))
    .unwrap()
    .encode();
    let mut evil = honest.clone();
    replace_tree(&mut evil, nums, vec![optional]);
    rejects(evil);
    let mut evil = honest;
    evil.descriptor = None;
    rejects(evil);
}

#[test]
fn emulated_covenants_are_replayed_using_the_recorded_threshold_and_roots() {
    let secp = Secp256k1::new();
    let roots: Vec<_> = [4, 5]
        .into_iter()
        .map(|byte| {
            Xpub::from_priv(
                &secp,
                &Xpriv::new_master(Network::Regtest, &[byte; 32]).unwrap(),
            )
        })
        .collect();
    for (signers, threshold) in [(vec![roots[0]], 1), (roots.clone(), 1), (roots.clone(), 2)] {
        let honest = payment(LoweringPlan::CtvEmulation { signers, threshold });
        honest.validate().unwrap();
        let (internal, leaves) = spending_data(&honest);
        let mut raw = honest.clone();
        replace_tree(&mut raw, internal, leaves.clone());
        raw.validate().unwrap();
        let mut evil = honest.clone();
        replace_tree(&mut evil, key(3), leaves);
        rejects(evil);
        let mut evil = honest;
        evil.covenant_requirements.lowering = LoweringPlan::Native;
        rejects(evil);
    }
}

#[test]
fn separately_declared_spending_authority_is_visible_and_is_not_producer_authentication() {
    let mut object = payment(LoweringPlan::Native);
    let (_, mut leaves) = spending_data(&object);
    let owner = Clause::Key(key(3));
    leaves.push(owner.compile::<Tap>().unwrap().encode());
    object.alternative_policies.push(owner.into());
    replace_tree(&mut object, key(3), leaves);
    object.validate().unwrap();
    let explanation = object.explain().unwrap();
    assert_eq!(
        explanation.nodes[0].alternative_policies,
        object.alternative_policies
    );
    // Dropping the declaration cannot silently retain its spending authority.
    object.alternative_policies.clear();
    rejects(object);

    // A plain payment destination makes no covenant claims.
    Object::from_descriptor(
        Descriptor::<XOnlyPublicKey>::from_str(&format!("tr({})", key(3))).unwrap(),
        Amount::ZERO,
    )
    .validate()
    .unwrap();
}

#[test]
fn claiming_an_unconditional_alternative_does_not_make_it_valid() {
    let mut object = payment(LoweringPlan::Native);
    let (nums, mut leaves) = spending_data(&object);
    leaves.push(ScriptBuf::from(vec![0x51]));
    object.alternative_policies.push(Clause::Trivial.into());
    replace_tree(&mut object, nums, leaves);
    rejects(object);
}

#[test]
fn native_descriptor_representation_is_checked_as_well_as_raw_taproot() {
    let mut object = payment(LoweringPlan::Native);
    let (nums, _) = spending_data(&object);
    let evil =
        Descriptor::<XOnlyPublicKey>::from_str(&format!("tr({nums},pk({}))", key(3))).unwrap();
    object.address = evil.address(Network::Regtest).unwrap().into();
    object.descriptor = Some(SupportedDescriptors::XOnly(evil));
    rejects(object);
}

#[test]
fn committed_branch_records_are_required_and_must_match_template_guards() {
    let honest = payment(LoweringPlan::Native);
    let mut evil = honest.clone();
    evil.committed_policy_guards.clear();
    rejects(evil);
    let mut evil = honest.clone();
    evil.ctv_to_tx
        .values_mut()
        .next()
        .unwrap()
        .guards
        .push(Clause::Key(key(3)).into());
    rejects(evil);
    let mut evil = honest;
    evil.ctv_to_tx.clear();
    evil.covenant_requirements.predicates.clear();
    rejects(evil);
}

#[test]
fn compiler_retains_lowering_records_for_unreachable_predicates() {
    use bitcoin::hashes::{sha256, Hash};
    use sapio::contract::{actions::Guard, DynamicContract};
    use sapio_base::policy::ScriptPolicy;
    use sapio_base::{Ctv, Emulatable};
    fn policy() -> Option<Guard<ScriptPolicy>> {
        Some(Guard::CachedPolicy(|source| Ok(source.clone()), None))
    }
    let predicate = Ctv(sha256::Hash::hash(b"unreachable predicate"));
    let object = DynamicContract {
        actions: vec![],
        finish: vec![policy],
        ensure_amount_f: Box::new(|_, _| Ok(Amount::ZERO)),
        metadata_f: Box::new(|_, _| Ok(Default::default())),
        data: ScriptPolicy::Or(vec![
            ScriptPolicy::And(vec![
                Clause::Unsatisfiable.into(),
                Emulatable(predicate).into(),
            ]),
            Clause::Key(key(3)).into(),
        ]),
    }
    .compile(Context::new(
        Network::Regtest,
        Amount::ZERO,
        LoweringPlan::Native,
        "unused_predicate".try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    ))
    .unwrap();
    assert!(object.covenant_requirements.predicates.contains(&predicate));
    object.validate().unwrap();
}
