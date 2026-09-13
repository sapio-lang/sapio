use super::*;
use crate::contract::actions::Guard;
use crate::contract::object::Object;
use crate::contract::Contract;
use bitcoin::bip32::{Xpriv, Xpub};
use bitcoin::blockdata::opcodes::all::OP_VERIFY;
use bitcoin::blockdata::script::Builder;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{Amount, Network, ScriptBuf};
use sapio_base::policy::ScriptFragment;
use sapio_base::program::{EmulatedProgram, ProgramInstance};
use sapio_base::timelocks::RelHeight;
use sapio_base::LoweringPlan;

struct PolicyContract {
    policies: [ScriptPolicy; 2],
    pinned: Option<XOnlyPublicKey>,
}

impl PolicyContract {
    fn first() -> Option<Guard<Self>> {
        Some(Guard::CachedPolicy(
            |contract| Ok(contract.policies[0].clone()),
            None,
        ))
    }
    fn second() -> Option<Guard<Self>> {
        Some(Guard::CachedPolicy(
            |contract| Ok(contract.policies[1].clone()),
            None,
        ))
    }
}

impl Contract for PolicyContract {
    crate::declare! {finish, Self::first, Self::second}

    fn pinned_internal_key(&self, _: &Context) -> Result<Option<XOnlyPublicKey>, CompilationError> {
        Ok(self.pinned)
    }
}

fn program(seed: u8) -> EmulatedProgram {
    let root = Xpub::from_priv(
        &Secp256k1::new(),
        &Xpriv::new_master(Network::Regtest, &[seed; 32]).unwrap(),
    );
    EmulatedProgram::new(
        ProgramInstance::wasm_v2(vec![seed], vec![seed + 1]).unwrap(),
        root,
    )
    .unwrap()
}

fn compile(policies: [ScriptPolicy; 2], pinned: Option<XOnlyPublicKey>) -> Object {
    PolicyContract { policies, pinned }
        .compile(Context::new(
            Network::Regtest,
            Amount::from_sat(1_000),
            LoweringPlan::Native,
            "program_policies".try_into().unwrap(),
            Arc::new(Default::default()),
            None,
        ))
        .unwrap()
}

fn single(policy: ScriptPolicy) -> Object {
    compile([policy, ScriptPolicy::Or(vec![])], None)
}

fn path(policy: &ScriptPolicy) -> ProgramSpendPath {
    ProgramSpendPath::ScriptPath(TapLeafHash::from_script(
        &compile_policy_leaf(policy).unwrap(),
        LeafVersion::TapScript,
    ))
}

#[test]
fn bare_program_retains_both_automatic_paths_but_pinning_removes_its_leaf() {
    let program = program(1);
    let source = ScriptPolicy::Program(program.clone());
    let automatic = single(source.clone());
    assert_eq!(
        automatic.program_policies,
        vec![ProgramPolicy {
            policy: source.clone(),
            paths: [ProgramSpendPath::KeyPath, path(&source)].into(),
        }]
    );
    let pinned = compile(
        [source.clone(), source.clone()],
        Some(program.derive_public_key().unwrap()),
    );
    assert_eq!(
        pinned.program_policies,
        vec![ProgramPolicy {
            policy: source,
            paths: [ProgramSpendPath::KeyPath].into(),
        }]
    );
    assert_eq!(pinned.program_requirements().unwrap().len(), 1);
}

#[test]
fn alternatives_keep_their_own_programs_and_native_guards() {
    let first = program(2);
    let left = program(3);
    let right = program(4);
    let source = ScriptPolicy::And(vec![
        first.clone().into(),
        Clause::try_from(RelHeight::from(6)).unwrap().into(),
        ScriptPolicy::Or(vec![left.clone().into(), right.clone().into()]),
    ]);
    let object = single(source);
    assert_eq!(object.program_policies.len(), 2);
    let requirements = object.program_requirements().unwrap();
    assert_eq!(requirements.len(), 4);
    for alternative in [left, right] {
        let policy = ScriptPolicy::And(vec![
            first.clone().into(),
            Clause::try_from(RelHeight::from(6)).unwrap().into(),
            alternative.clone().into(),
        ]);
        let location = path(&policy);
        assert_eq!(
            requirements
                .iter()
                .filter(|requirement| requirement.path == location)
                .map(|requirement| requirement.program.clone())
                .collect::<BTreeSet<_>>(),
            [first.clone(), alternative].into()
        );
    }
    assert!(requirements
        .iter()
        .all(|requirement| requirement.path != ProgramSpendPath::KeyPath));
    let decoded: Object = serde_json::from_value(serde_json::to_value(&object).unwrap()).unwrap();
    assert_eq!(decoded.program_requirements().unwrap(), requirements);
}

#[test]
fn native_disjunction_inside_a_guard_stays_in_its_complete_leaf() {
    let program = program(5);
    let choice = Clause::Or(vec![
        (
            1,
            Arc::new(Clause::Key(self::program(21).derive_public_key().unwrap())),
        ),
        (
            1,
            Arc::new(Clause::Key(self::program(22).derive_public_key().unwrap())),
        ),
    ]);
    let source = ScriptPolicy::And(vec![program.clone().into(), choice.clone().into()]);
    let expected =
        conjoin_guards([&Clause::Key(program.derive_public_key().unwrap()), &choice].into_iter())
            .compile::<Tap>()
            .unwrap()
            .encode();
    assert_eq!(compile_policy_leaf(&source).unwrap(), expected);
    let object = single(source.clone());
    assert_eq!(
        object.program_policies,
        vec![ProgramPolicy {
            policy: source.clone(),
            paths: [path(&source)].into()
        }]
    );
    assert!(matches!(
        object.descriptor,
        Some(SupportedDescriptors::XOnly(_))
    ));
}

#[test]
fn raw_composition_retains_program_order_and_exact_leaf_provenance() {
    let left = program(6);
    let right = program(7);
    let raw = ScriptFragment::new(Builder::new().push_int(1).into_script()).unwrap();
    let source = ScriptPolicy::And(vec![left.clone().into(), raw.into(), right.clone().into()]);
    let mut expected = Clause::Key(left.derive_public_key().unwrap())
        .compile::<Tap>()
        .unwrap()
        .encode()
        .into_bytes();
    expected.extend_from_slice(&[OP_VERIFY.to_u8(), 0x51, OP_VERIFY.to_u8()]);
    expected.extend_from_slice(
        Clause::Key(right.derive_public_key().unwrap())
            .compile::<Tap>()
            .unwrap()
            .encode()
            .as_bytes(),
    );
    assert_eq!(
        compile_policy_leaf(&source).unwrap(),
        ScriptBuf::from(expected)
    );
    let object = single(source.clone());
    assert_eq!(
        object.program_policies,
        vec![ProgramPolicy {
            policy: source.clone(),
            paths: [path(&source)].into()
        }]
    );
    assert_eq!(object.program_requirements().unwrap().len(), 2);
}

#[test]
fn identical_remaining_raw_leaf_is_still_a_program_location_after_pinning() {
    let program = program(8);
    let source = ScriptPolicy::Program(program.clone());
    let raw = ScriptFragment::new(compile_policy_leaf(&source).unwrap())
        .unwrap()
        .into();
    let object = compile(
        [source.clone(), raw],
        Some(program.derive_public_key().unwrap()),
    );
    assert_eq!(
        object.program_policies,
        vec![ProgramPolicy {
            policy: source.clone(),
            paths: [ProgramSpendPath::KeyPath, path(&source)].into(),
        }]
    );
}

#[test]
fn independent_sources_survive_identical_leaf_deduplication() {
    let program = program(9);
    let source = ScriptPolicy::Program(program.clone());
    let repeated = ScriptPolicy::And(vec![source.clone(), source.clone()]);
    let object = compile([source.clone(), repeated.clone()], None);
    assert_eq!(object.program_policies.len(), 2);
    assert_eq!(object.program_requirements().unwrap().len(), 2);
    assert!(object
        .program_policies
        .iter()
        .all(|record| record.paths == [ProgramSpendPath::KeyPath, path(&source)].into()));
}
