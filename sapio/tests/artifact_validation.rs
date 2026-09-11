use bitcoin::hashes::{sha256, Hash};
use bitcoin::psbt::Psbt;
use bitcoin::{Address, Amount, Network, OutPoint, ScriptBuf, Transaction, Txid, Witness};
use sapio::contract::abi::object::{ArtifactErrorKind, ObjectError};
use sapio::contract::abi::studio::SapioStudioFormat;
use sapio::contract::{Compilable, Compiled, Context, Contract};
use sapio::template::Template;
use sapio::{declare, then};
use sapio_base::covenant::LoweringPlan;
use sapio_base::txindex::{TxIndex, TxIndexError, TxIndexLogger};
use sapio_base::util::CTVHash;
use sapio_base::Clause;
use sapio_ctv_emulator_trait::{CTVAvailable, CTVEmulator, EmulatorError};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;

struct Payment {
    destination: Compiled,
    extra_input: bool,
}

impl Payment {
    #[then]
    fn pay(self, ctx: Context) {
        let mut builder =
            ctx.template()
                .add_output(Amount::from_sat(1_000), &self.destination, None)?;
        if self.extra_input {
            builder = builder.add_sequence();
        }
        builder.into()
    }
}

impl Contract for Payment {
    declare! {then, Self::pay}
    declare! {non updatable}
}

fn payment(destination: Compiled, extra_input: bool, path: &str) -> Compiled {
    Payment {
        destination,
        extra_input,
    }
    .compile(Context::new(
        Network::Regtest,
        Amount::from_sat(1_000),
        LoweringPlan::Native,
        path.try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    ))
    .unwrap()
}

fn leaf() -> Compiled {
    Compiled::from_address(
        Address::from_str("bcrt1qumrrqgt7e3a7damzm8x97m6sjs20u8hjw2hcjj")
            .unwrap()
            .require_network(Network::Regtest)
            .unwrap(),
        bitcoin::Amount::ZERO,
    )
}

fn wire_roundtrip(object: Compiled) -> Compiled {
    serde_json::from_slice(&serde_json::to_vec(&object).unwrap()).unwrap()
}

fn template(object: &mut Compiled) -> &mut Template {
    object.ctv_to_tx.values_mut().next().unwrap()
}

// A malformed graph must fail before even looking up a UTXO or asking a signer.
struct NoEffects;

impl TxIndex for NoEffects {
    fn lookup_tx(&self, _: &Txid) -> Result<Arc<Transaction>, TxIndexError> {
        panic!("invalid artifact reached transaction lookup");
    }
    fn add_tx(&self, _: Arc<Transaction>) -> Result<Txid, TxIndexError> {
        panic!("invalid artifact reached transaction insertion");
    }
}

impl CTVEmulator for NoEffects {
    fn get_signer_for(&self, _: sha256::Hash) -> Result<Clause, EmulatorError> {
        panic!("invalid artifact requested a signer");
    }
    fn sign(&self, _: Psbt) -> Result<Psbt, EmulatorError> {
        panic!("invalid artifact reached signing");
    }
}

fn reject_artifact(object: Compiled, expected: ArtifactErrorKind) {
    let object = wire_roundtrip(object);
    let error = object
        .bind_psbt(
            OutPoint::default(),
            BTreeMap::new(),
            Rc::new(NoEffects),
            &NoEffects,
        )
        .expect_err("malformed artifact was accepted");
    let ObjectError::InvalidArtifact(error) = error else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(error.kind, expected);
}

#[test]
fn rejects_malformed_unsigned_templates_at_the_binding_boundary() {
    type Mutation = fn(&mut Template);
    let cases: [(Mutation, ArtifactErrorKind); 13] = [
        (|t| t.tx.input.clear(), ArtifactErrorKind::MissingInput),
        (
            |t| t.ctv_index = 1,
            ArtifactErrorKind::UnsupportedInputIndex(1),
        ),
        (|t| t.inputs.clear(), ArtifactErrorKind::InputMetadataCount),
        (
            |t| t.outputs.clear(),
            ArtifactErrorKind::OutputMetadataCount,
        ),
        (
            |t| t.tx.input[0].script_sig = ScriptBuf::from(vec![0x51]),
            ArtifactErrorKind::SignedInput(0),
        ),
        (
            |t| t.tx.input[0].witness = Witness::from_slice(&[vec![1]]),
            ArtifactErrorKind::SignedInput(0),
        ),
        (
            |t| t.ctv = sha256::Hash::from_slice(&[0; 32]).unwrap(),
            ArtifactErrorKind::TemplateHashMismatch,
        ),
        (
            |t| {
                t.tx.lock_time = bitcoin::absolute::LockTime::from_consensus(
                    t.tx.lock_time.to_consensus_u32() + 1,
                )
            },
            ArtifactErrorKind::TemplateHashMismatch,
        ),
        (
            |t| t.outputs[0].amount = Amount::from_sat(999),
            ArtifactErrorKind::OutputMismatch(0),
        ),
        (
            |t| {
                t.outputs[0].contract.address =
                    sapio::util::extended_address::ExtendedAddress::Unknown(ScriptBuf::new())
            },
            ArtifactErrorKind::OutputMismatch(0),
        ),
        (
            |t| t.max = Amount::from_sat(999),
            ArtifactErrorKind::InsufficientAmount,
        ),
        (
            |t| t.required_input_amount = Amount::from_sat(1_001),
            ArtifactErrorKind::InvalidInputAmount,
        ),
        (
            |t| t.required_input_amount = Amount::from_sat(999),
            ArtifactErrorKind::InvalidInputAmount,
        ),
    ];
    for (mutate, expected) in cases {
        let mut object = payment(leaf(), false, "payment");
        mutate(template(&mut object));
        reject_artifact(object, expected);
    }
}

#[test]
fn funding_requirements_are_mandatory_in_serialized_templates() {
    let mut object = payment(leaf(), false, "payment");
    let mut value = serde_json::to_value(template(&mut object)).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("required_input_amount_sats");
    assert!(serde_json::from_value::<Template>(value).is_err());
}

#[test]
fn rejects_inconsistent_map_keys_and_descriptors() {
    let mut object = payment(leaf(), false, "payment");
    let (_, t) = object.ctv_to_tx.pop_first().unwrap();
    object
        .ctv_to_tx
        .insert(sha256::Hash::from_slice(&[0; 32]).unwrap(), t);
    reject_artifact(object, ArtifactErrorKind::TemplateHashMismatch);

    let mut object = payment(leaf(), false, "payment");
    object.address = sapio::util::extended_address::ExtendedAddress::Unknown(ScriptBuf::new());
    reject_artifact(object, ArtifactErrorKind::DescriptorMismatch);
}

#[test]
fn rejects_overflow_even_when_output_metadata_and_hashes_agree() {
    let mut object = payment(leaf(), false, "payment");
    let (_, mut t) = object.ctv_to_tx.pop_first().unwrap();
    t.tx.output[0].value = bitcoin::Amount::from_sat(u64::MAX);
    t.outputs[0].amount = Amount::from_sat(u64::MAX);
    t.tx.output.push(t.tx.output[0].clone());
    t.outputs.push(t.outputs[0].clone());
    t.max = Amount::from_sat(u64::MAX);
    t.ctv = t.tx.get_ctv_hash(0);
    object.ctv_to_tx.insert(t.ctv, t);
    reject_artifact(object, ArtifactErrorKind::OutputAmountOverflow);
}

#[test]
fn checks_nested_and_suggested_templates_before_any_side_effect() {
    let inner = payment(leaf(), false, "inner");
    let mut outer = payment(inner, false, "outer");
    let inner = &mut template(&mut outer).outputs[0].contract;
    let expected_path = inner.root_path.clone();
    template(inner).tx.input.clear();
    let error = outer.validate().unwrap_err();
    assert_eq!(error.path, expected_path);
    assert!(error.template.is_some());
    reject_artifact(outer, ArtifactErrorKind::MissingInput);

    let mut object = payment(leaf(), false, "suggested");
    object.suggested_txs = std::mem::take(&mut object.ctv_to_tx);

    object
        .suggested_txs
        .values_mut()
        .next()
        .unwrap()
        .tx
        .input
        .clear();
    reject_artifact(object, ArtifactErrorKind::MissingInput);
}

#[test]
fn rejects_invalid_input_mappings_before_binding() {
    let child = payment(leaf(), true, "child");
    let child_hash = *child.ctv_to_tx.keys().next().unwrap();
    let object = payment(child, false, "parent");
    for mapping in [
        BTreeMap::from([(child_hash, vec![None])]),
        BTreeMap::from([(child_hash, vec![None, None, None])]),
        BTreeMap::from([(child_hash, vec![Some(OutPoint::default()), None])]),
        BTreeMap::from([(sha256::Hash::from_slice(&[0; 32]).unwrap(), vec![None])]),
    ] {
        assert!(matches!(
            object.bind_psbt(OutPoint::default(), mapping, Rc::new(NoEffects), &NoEffects),
            Err(ObjectError::InvalidInputMapping { .. })
        ));
    }
}

#[test]
fn binds_roundtripped_artifacts_with_explicit_auxiliary_inputs() {
    let object = wire_roundtrip(payment(leaf(), true, "payment"));
    object.validate().unwrap();
    let hash = *object.ctv_to_tx.keys().next().unwrap();
    let contract_input = OutPoint::new(Txid::from_slice(&[0; 32]).unwrap(), 2);
    let auxiliary_input = OutPoint::new(Txid::from_slice(&[0; 32]).unwrap(), 3);
    let program = object
        .bind_psbt(
            contract_input,
            BTreeMap::from([(hash, vec![None, Some(auxiliary_input)])]),
            Rc::new(TxIndexLogger::new()),
            &CTVAvailable,
        )
        .unwrap();
    let txs = &program.program.get(&object.root_path).unwrap().txs;
    assert_eq!(txs.len(), 1);
    let SapioStudioFormat::LinkedPSBT { psbt, .. } = &txs[0];
    let psbt: Psbt = Psbt::deserialize(&base64::decode(psbt).unwrap()).unwrap();
    assert_eq!(psbt.unsigned_tx.input[0].previous_output, contract_input);
    assert_eq!(psbt.unsigned_tx.input[1].previous_output, auxiliary_input);
    assert_eq!(psbt.unsigned_tx.get_ctv_hash(0), hash);
    assert_eq!(psbt.unsigned_tx.output[0].value.to_sat(), 1_000);
}
