use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::Secp256k1;
use bitcoin::util::bip32::{ChildNumber, ExtendedPrivKey, ExtendedPubKey};
use bitcoin::Network;
use sapio_base::covenant::hash_to_child_vec;
use sapio_base::policy::{PolicyCompiler, ScriptPolicy};
use sapio_base::program::{
    program_derivation_path, ProgramError, MAX_PARAMETER_BYTES, MAX_PROGRAM_BYTES,
    MAX_PROGRAM_ROOT_DEPTH,
};
use sapio_base::{Clause, EmulatedProgram, EvaluatorId, ProgramId, ProgramInstance};
use serde_json::json;
use std::collections::BTreeSet;
use std::str::FromStr;

fn evaluator() -> EvaluatorId {
    EvaluatorId(sha256::Hash::hash(b"test evaluator v1"))
}

fn instance() -> ProgramInstance {
    ProgramInstance::new(
        evaluator(),
        b"payment".to_vec(),
        b"recipient and minimum".to_vec(),
    )
    .unwrap()
}

fn root(byte: u8) -> ExtendedPrivKey {
    ExtendedPrivKey::new_master(Network::Testnet, &[byte; 32]).unwrap()
}

fn public_root(byte: u8) -> ExtendedPubKey {
    ExtendedPubKey::from_priv(&Secp256k1::new(), &root(byte))
}

#[test]
fn wasm_identities_distinguish_inline_programs_from_registered_interpreters() {
    let module = b"\0asm\x01\0\0\0";
    let id = EvaluatorId::for_wasm(module);
    assert_eq!(
        id.0.to_string(),
        "56266469888468b6f9b89684a92561dc98943fa8fb88c35eebd37f2e16a6b747"
    );
    assert!(!id.is_wasm());
    assert_eq!(
        EvaluatorId::default(),
        EvaluatorId(sha256::Hash::from_inner([0; 32]))
    );
    let inline = ProgramInstance::wasm(module.to_vec(), vec![7]).unwrap();
    assert!(inline.evaluator().is_wasm());
    let interpreted = ProgramInstance::new(id, module.to_vec(), vec![7]).unwrap();
    assert_ne!(inline.id(), interpreted.id());
    assert_ne!(id, EvaluatorId::for_wasm(b"\0asm\x01\0\0\0\0"));
}

#[test]
fn exact_instance_commitment_matches_independent_tagged_hash_vectors() {
    assert_eq!(
        evaluator().0.to_string(),
        "c8f9fd14473d8d0bc63e497ff1ab455a54ea2be63bd8e4f560ca871c180e9ac3"
    );
    assert_eq!(
        instance().id(),
        ProgramId(
            sha256::Hash::from_str(
                "609d3c0765d9092356f0345863c28398291401d6f50bae2d541ec6dda5c40a12"
            )
            .unwrap()
        )
    );
    let empty = ProgramInstance::new(
        EvaluatorId(sha256::Hash::from_inner([0; 32])),
        vec![],
        vec![],
    )
    .unwrap();
    assert_eq!(
        empty.id().0.to_string(),
        "67a574489f81a569a969df673af1e620ce798def9e49d5349cbac171be3b567d"
    );
}

#[test]
fn every_identity_component_and_byte_boundary_changes_the_commitment() {
    let base = instance();
    let variations = [
        base.clone(),
        ProgramInstance::new(
            EvaluatorId(sha256::Hash::hash(b"test evaluator v2")),
            base.program().to_vec(),
            base.parameters().to_vec(),
        )
        .unwrap(),
        ProgramInstance::new(evaluator(), b"Payment".to_vec(), base.parameters().to_vec()).unwrap(),
        ProgramInstance::new(
            evaluator(),
            base.program().to_vec(),
            b"new recipient".to_vec(),
        )
        .unwrap(),
        ProgramInstance::new(evaluator(), b"ab".to_vec(), b"c".to_vec()).unwrap(),
        ProgramInstance::new(evaluator(), b"a".to_vec(), b"bc".to_vec()).unwrap(),
        ProgramInstance::new(evaluator(), vec![], b"abc".to_vec()).unwrap(),
        ProgramInstance::new(evaluator(), b"abc".to_vec(), vec![]).unwrap(),
    ];
    let ids: BTreeSet<_> = variations.iter().map(ProgramInstance::id).collect();
    assert_eq!(ids.len(), variations.len());
    let keys: BTreeSet<_> = variations
        .iter()
        .map(|instance| instance.derive_public_key(&public_root(7)).unwrap())
        .collect();
    assert_eq!(keys.len(), variations.len());
}

#[test]
fn program_derivation_has_a_distinct_namespace_and_agrees_with_private_keys() {
    let secp = Secp256k1::new();
    let instance = instance();
    let path = program_derivation_path(instance.id());
    assert_eq!(path.len(), 10);
    assert_eq!(path[0], ChildNumber::Normal { index: 0x5341_5049 });
    assert!(path.iter().all(ChildNumber::is_normal));
    assert_eq!(path[1..], hash_to_child_vec(instance.id().0));
    for byte in [7, 8] {
        let private = root(byte).derive_priv(&secp, &path).unwrap();
        let public = instance.derive_public_key(&public_root(byte)).unwrap();
        assert_eq!(public, private.to_keypair(&secp).x_only_public_key().0);
        let legacy_ctv = public_root(byte)
            .derive_pub(&secp, &hash_to_child_vec(instance.id().0))
            .unwrap()
            .to_x_only_pub();
        assert_ne!(public, legacy_ctv);
    }
    assert_ne!(
        instance.derive_public_key(&public_root(7)).unwrap(),
        instance.derive_public_key(&public_root(8)).unwrap()
    );
}

#[test]
fn byte_limits_apply_to_native_construction_and_bounded_deserialization() {
    let maximum = ProgramInstance::new(
        evaluator(),
        vec![1; MAX_PROGRAM_BYTES],
        vec![2; MAX_PARAMETER_BYTES],
    )
    .unwrap();
    assert_eq!(
        serde_json::from_value::<ProgramInstance>(serde_json::to_value(&maximum).unwrap()).unwrap(),
        maximum
    );
    assert_eq!(
        ProgramInstance::new(evaluator(), vec![1; MAX_PROGRAM_BYTES + 1], vec![]),
        Err(ProgramError::ProgramTooLarge {
            size: MAX_PROGRAM_BYTES + 1
        })
    );
    assert_eq!(
        ProgramInstance::new(evaluator(), vec![], vec![2; MAX_PARAMETER_BYTES + 1]),
        Err(ProgramError::ParametersTooLarge {
            size: MAX_PARAMETER_BYTES + 1
        })
    );
    for (field, maximum) in [
        ("program", MAX_PROGRAM_BYTES),
        ("parameters", MAX_PARAMETER_BYTES),
    ] {
        let mut value = serde_json::to_value(instance()).unwrap();
        value[field] = json!(vec![0; maximum + 1]);
        assert!(serde_json::from_value::<ProgramInstance>(value).is_err());
    }
}

#[test]
fn malformed_or_incomplete_serialized_instances_are_rejected() {
    for field in ["evaluator", "program", "parameters"] {
        let mut value = serde_json::to_value(instance()).unwrap();
        value.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<ProgramInstance>(value).is_err());
    }
    for (field, malformed) in [
        ("evaluator", json!("01")),
        ("program", json!([256])),
        ("parameters", json!([-1])),
        ("uncommitted_parameters", json!([])),
    ] {
        let mut value = serde_json::to_value(instance()).unwrap();
        value[field] = malformed;
        assert!(serde_json::from_value::<ProgramInstance>(value).is_err());
    }
}

#[test]
fn root_depth_is_checked_in_public_derivation_and_policy_deserialization() {
    let instance = instance();
    let mut root = public_root(7);
    root.depth = MAX_PROGRAM_ROOT_DEPTH;
    instance.derive_public_key(&root).unwrap();
    let maximum = EmulatedProgram::new(instance.clone(), root).unwrap();
    serde_json::from_value::<EmulatedProgram>(serde_json::to_value(maximum).unwrap()).unwrap();
    for depth in [MAX_PROGRAM_ROOT_DEPTH + 1, 255] {
        root.depth = depth;
        assert_eq!(
            instance.derive_public_key(&root),
            Err(ProgramError::RootDepth { depth })
        );
        assert_eq!(
            EmulatedProgram::new(instance.clone(), root),
            Err(ProgramError::RootDepth { depth })
        );
        assert!(serde_json::from_value::<EmulatedProgram>(json!({
            "instance": instance,
            "root": root.to_string(),
        }))
        .is_err());
    }
}

#[test]
fn serialized_public_policy_retains_its_complete_instance_and_lowers_to_a_key() {
    let instance = instance();
    let policy = EmulatedProgram::new(instance.clone(), public_root(7)).unwrap();
    let json = serde_json::to_value(&policy).unwrap();
    assert_eq!(
        json,
        json!({
            "instance": {
                "evaluator": evaluator().0.to_string(),
                "program": instance.program(),
                "parameters": instance.parameters(),
            },
            "root": public_root(7).to_string(),
        })
    );
    let restored: EmulatedProgram = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(restored, policy);
    assert_eq!(restored.instance(), &instance);
    assert_eq!(restored.root(), &public_root(7));
    assert_eq!(
        restored.compile_policy().unwrap(),
        ScriptPolicy::Miniscript(Clause::Key(policy.derive_public_key().unwrap()))
    );
    let mut extra_field = json.clone();
    extra_field["endpoint"] = json!("ambient runtime");
    assert!(serde_json::from_value::<EmulatedProgram>(extra_field).is_err());
    let mut bad_key = json;
    bad_key["root"] = json!("invalid root");
    assert!(serde_json::from_value::<EmulatedProgram>(bad_key).is_err());
    let schema = serde_json::to_value(schemars::schema_for!(ProgramInstance)).unwrap();
    assert_eq!(
        schema["properties"]["program"]["maxItems"],
        MAX_PROGRAM_BYTES
    );
    assert_eq!(
        schema["properties"]["parameters"]["maxItems"],
        MAX_PARAMETER_BYTES
    );
    serde_json::to_value(schemars::schema_for!(EmulatedProgram)).unwrap();
}
