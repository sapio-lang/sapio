use bitcoin::bip32::{Xpriv, Xpub};
use bitcoin::hashes::Hash;
use bitcoin::taproot::TapLeafHash;
use bitcoin::{Amount, Network};
use sapio::contract::abi::object::{ArtifactErrorKind, ObjectMetadata};
use sapio::contract::{Compilable, Compiled, Context, Contract};
use sapio::{declare, guard, then};
use sapio_base::fragments::{template_signed_by, TemplateKey};
use sapio_base::policy::ScriptPolicy;
use sapio_base::program::{EmulatedProgram, ProgramSpendPath};
use sapio_base::timelocks::RelHeight;
use sapio_base::{Clause, LoweringPlan};
use std::sync::Arc;

fn program(seed: u8) -> EmulatedProgram {
    let secret = Xpriv::new_master(Network::Regtest, &[seed; 32]).unwrap();
    template_signed_by(
        TemplateKey::InternalKey,
        Xpub::from_priv(&bitcoin::secp256k1::Secp256k1::new(), &secret),
    )
    .unwrap()
}

fn guarded(program: &EmulatedProgram, delay: u16) -> ScriptPolicy {
    ScriptPolicy::And(vec![
        Clause::try_from(RelHeight::from(delay)).unwrap().into(),
        program.clone().into(),
    ])
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

fn context() -> Context {
    Context::new(
        Network::Regtest,
        Amount::from_sat(1_000),
        LoweringPlan::Native,
        "program_artifact".try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    )
}

fn compile(policy: ScriptPolicy) -> Compiled {
    Source(policy).compile(context()).unwrap()
}

fn invalid(object: &Compiled) {
    assert!(matches!(
        object.validate().unwrap_err().kind,
        ArtifactErrorKind::InvalidProgramPolicy(_)
    ));
}

#[test]
fn alternatives_and_joint_guards_keep_their_exact_program_slots() {
    let first = program(21);
    let second = program(22);
    // An instance ID alone does not distinguish independent oracle roots.
    assert_eq!(first.instance().id(), second.instance().id());
    let object = compile(ScriptPolicy::Or(vec![
        guarded(&first, 6),
        guarded(&first, 12),
        guarded(&second, 6),
        guarded(&first, 6),
    ]));
    let expected = object.program_requirements().unwrap();
    assert_eq!(expected.len(), 3);
    assert_eq!(
        expected.iter().filter(|slot| slot.program == first).count(),
        2
    );
    assert_eq!(
        expected
            .iter()
            .filter(|slot| slot.program == second)
            .count(),
        1
    );
    assert!(expected
        .iter()
        .all(|slot| matches!(slot.path, ProgramSpendPath::ScriptPath(_))));

    let mut decoded: Compiled =
        serde_json::from_slice(&serde_json::to_vec(&object).unwrap()).unwrap();
    decoded.metadata = ObjectMetadata::default();
    assert_eq!(decoded.program_requirements().unwrap(), expected);
    assert_eq!(decoded.descriptor, object.descriptor);

    let joint = compile(ScriptPolicy::And(vec![guarded(&first, 6), second.into()]));
    let slots = joint.program_requirements().unwrap();
    assert_eq!(slots.len(), 2);
    assert_eq!(
        slots
            .iter()
            .map(|slot| slot.path)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        1
    );
}

#[test]
fn changing_program_source_native_guards_or_locations_invalidates_the_artifact() {
    let original_program = program(23);
    let original = compile(guarded(&original_program, 6));
    for policy in [
        guarded(&program(24), 6),
        guarded(&original_program, 7),
        Clause::Key(original_program.derive_public_key().unwrap()).into(),
    ] {
        let mut changed = original.clone();
        changed.program_policies[0].policy = policy;
        invalid(&changed);
    }
    for paths in [
        Default::default(),
        [ProgramSpendPath::KeyPath].into(),
        [ProgramSpendPath::ScriptPath(TapLeafHash::from_byte_array(
            [42; 32],
        ))]
        .into(),
    ] {
        let mut changed = original.clone();
        changed.program_policies[0].paths = paths;
        invalid(&changed);
    }
    let mut duplicate = original.clone();
    duplicate
        .program_policies
        .push(duplicate.program_policies[0].clone());
    invalid(&duplicate);

    let bare = compile(original_program.into());
    assert_eq!(bare.program_requirements().unwrap().len(), 2);
    let mut incomplete = bare;
    assert!(incomplete.program_policies[0]
        .paths
        .remove(&ProgramSpendPath::KeyPath));
    invalid(&incomplete);
}

struct Parent(Compiled);
impl Parent {
    #[then]
    fn advance(self, ctx: Context) {
        ctx.template()
            .add_output(Amount::from_sat(1_000), &self.0, None)?
            .into()
    }
}
impl Contract for Parent {
    declare! {actions, Self::advance}
}

#[test]
fn program_validation_reaches_descendants_and_requires_the_wire_field() {
    let child = compile(guarded(&program(25), 6));
    let mut wire = serde_json::to_value(&child).unwrap();
    wire.as_object_mut().unwrap().remove("program_policies");
    assert!(serde_json::from_value::<Compiled>(wire).is_err());
    let mut parent = Parent(child).compile(context()).unwrap();
    parent.validate().unwrap();
    parent.ctv_to_tx.values_mut().next().unwrap().outputs[0]
        .contract
        .program_policies[0]
        .paths
        .clear();
    invalid(&parent);
}

#[test]
fn dead_source_alternatives_cannot_add_signature_slots_to_raw_leaves() {
    use bitcoin::{ScriptBuf, XOnlyPublicKey};
    use sapio_base::miniscript::{Miniscript, Tap};
    use sapio_base::policy::ScriptFragment;

    let raw = ScriptFragment::new(ScriptBuf::from(vec![0x51, 0x76, 0x93, 0x52, 0x87])).unwrap();
    let mut object = compile(ScriptPolicy::And(vec![program(26).into(), raw.into()]));
    let original = object.program_policies[0].policy.clone();
    let script = sapio::contract::compiler::compile_policy_leaf(&original).unwrap();
    assert!(Miniscript::<XOnlyPublicKey, Tap>::decode_consensus(&script).is_err());
    assert_eq!(object.program_requirements().unwrap().len(), 1);

    // The dead alternative adds no script bytes, but syntactic collection
    // would incorrectly advertise its program as a live signature slot.
    object.program_policies[0].policy = ScriptPolicy::Or(vec![
        original,
        ScriptPolicy::And(vec![ScriptPolicy::Or(vec![]), program(27).into()]),
    ]);
    invalid(&object);
}
