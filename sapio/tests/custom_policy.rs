#[path = "fixtures/custom_policy.rs"]
mod fixture;

use bitcoin::consensus::deserialize;
use bitcoin::psbt::PartiallySignedTransaction as Psbt;
use bitcoin::{Amount, OutPoint};
use fixture::*;
use sapio::contract::abi::studio::SapioStudioFormat;
use sapio::contract::{Compilable, CompilationError, Compiled, Context, Contract};
use sapio::{declare, guard};
use sapio_base::miniscript::{Miniscript, Tap};
use sapio_base::policy::{PolicyCompiler, PolicyError, ScriptPolicy};
use sapio_base::simp::{GuardLT, SIMPAttachableAt};
use sapio_base::txindex::{TxIndex, TxIndexLogger};
use sapio_base::util::CTVHash;
use sapio_base::Clause;
use sapio_ctv_emulator_trait::CTVAvailable;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;

#[test]
fn an_external_backend_compiles_beyond_miniscript_and_round_trips() {
    let compiled = compiled(false, false);
    let raw = tree(&compiled);
    assert_eq!(raw.leaves().len(), 1);
    let script = &raw.leaves()[0].1;
    assert!(Miniscript::<bitcoin::XOnlyPublicKey, Tap>::parse(script).is_err());
    assert_eq!(script_signers(script), vec![key(1)]);
    let decoded: Compiled =
        serde_json::from_slice(&serde_json::to_vec(&compiled).unwrap()).unwrap();
    decoded.validate().unwrap();
    assert_eq!(decoded, compiled);
    assert_ne!(raw.internal_key(), key(1));
    let spend = signed_spend(
        &compiled,
        unsigned_spend(&compiled, OutPoint::default()),
        None,
        false,
    );
    assert_eq!(spend.input[0].witness.len(), 3);
    let control = raw
        .spend_info()
        .control_block(&(
            script.clone(),
            bitcoin::util::taproot::LeafVersion::TapScript,
        ))
        .unwrap();
    assert!(control.verify_taproot_commitment(
        &bitcoin::secp256k1::Secp256k1::new(),
        raw.spend_info().output_key().to_inner(),
        script
    ));
}

#[test]
fn native_guards_custom_template_guards_and_covenants_survive_composition() {
    for emulated in [false, true] {
        let compiled = compiled(true, emulated);
        let raw = tree(&compiled);
        assert_eq!(raw.leaves().len(), 1);
        let script = &raw.leaves()[0].1;
        let expected: BTreeSet<_> = (1..=if emulated { 4 } else { 3 }).map(key).collect();
        assert_eq!(
            script_signers(script).into_iter().collect::<BTreeSet<_>>(),
            expected
        );
        for owner in [1, 3] {
            let ScriptPolicy::Script(fragment) =
                ArithmeticSigner(key(owner)).compile_policy().unwrap()
            else {
                unreachable!()
            };
            assert!(script
                .as_bytes()
                .windows(fragment.as_script().len())
                .any(|bytes| bytes == fragment.as_script().as_bytes()));
        }
        let template = compiled.ctv_to_tx.values().next().unwrap();
        assert_eq!(template.tx.output[0].value, 9_000);
        assert_eq!(template.required_input_amount, Amount::from_sat(10_000));
        if !emulated {
            let covenant = Clause::TxTemplate(template.tx.get_ctv_hash(0))
                .compile::<Tap>()
                .unwrap()
                .encode();
            assert!(script
                .as_bytes()
                .windows(covenant.len())
                .any(|bytes| bytes == covenant.as_bytes()));
        }
    }
}

#[test]
fn bound_raw_artifacts_provide_complete_psbt_spending_data() {
    let compiled = compiled(true, false);
    let tx = Arc::new(funding(&compiled));
    let index = Rc::new(TxIndexLogger::new());
    let txid = index.add_tx(tx.clone()).unwrap();
    let program = compiled
        .bind_psbt(
            OutPoint::new(txid, 0),
            BTreeMap::new(),
            index,
            &CTVAvailable,
        )
        .unwrap();
    let root = &program.program[&compiled.root_path];
    let SapioStudioFormat::LinkedPSBT { psbt, .. } = &root.txs[0];
    let psbt: Psbt = deserialize(&base64::decode(psbt).unwrap()).unwrap();
    assert_eq!(
        psbt.unsigned_tx.input[0].previous_output,
        OutPoint::new(txid, 0)
    );
    assert_eq!(psbt.inputs[0].witness_utxo.as_ref(), Some(&tx.output[0]));
    assert_eq!(psbt.inputs[0].non_witness_utxo.as_ref(), Some(tx.as_ref()));
    let raw = tree(&compiled);
    assert_eq!(psbt.inputs[0].tap_internal_key, Some(raw.internal_key()));
    assert_eq!(
        psbt.inputs[0].tap_merkle_root,
        raw.spend_info().merkle_root()
    );
    assert_eq!(psbt.inputs[0].tap_scripts.len(), raw.leaves().len());
    for (control, (script, _)) in &psbt.inputs[0].tap_scripts {
        assert!(control.verify_taproot_commitment(
            &bitcoin::secp256k1::Secp256k1::new(),
            raw.spend_info().output_key().to_inner(),
            script
        ));
    }
}

struct Nested;

impl Nested {
    #[guard(policy)]
    fn alternatives(self, _ctx: Context) -> ScriptPolicy {
        let raw = |owner| ArithmeticSigner(key(owner)).compile_policy().unwrap();
        ScriptPolicy::And(vec![
            ScriptPolicy::Or(vec![raw(1), ScriptPolicy::Or(vec![raw(2)])]),
            ScriptPolicy::Or(vec![raw(3), raw(4)]),
        ])
    }
}

impl Contract for Nested {
    declare! {finish, Self::alternatives}
    declare! {non updatable}
}

#[test]
fn nested_custom_alternatives_keep_each_required_pair() {
    let compiled = Nested.compile(context(false)).unwrap();
    let required: BTreeSet<_> = tree(&compiled)
        .leaves()
        .iter()
        .map(|(_, script)| script_signers(script))
        .collect();
    assert_eq!(
        required,
        BTreeSet::from([
            vec![key(1), key(3)],
            vec![key(1), key(4)],
            vec![key(2), key(3)],
            vec![key(2), key(4)]
        ])
    );
}

struct FallibleSigner(bool);

impl PolicyCompiler for FallibleSigner {
    fn compile_policy(&self) -> Result<ScriptPolicy, PolicyError> {
        if self.0 {
            Err(PolicyError::Backend("translation rejected".into()))
        } else {
            ArithmeticSigner(key(1)).compile_policy()
        }
    }
}

#[derive(Default)]
struct Cached {
    calls: Cell<usize>,
    paths: RefCell<Vec<String>>,
    backend_error: bool,
    metadata_error: bool,
}

impl Cached {
    #[guard(policy, cached, simps = "Some(Self::metadata)")]
    fn owner(self) -> FallibleSigner {
        self.calls.set(self.calls.get() + 1);
        FallibleSigner(self.backend_error)
    }

    fn metadata(
        &self,
        ctx: Context,
    ) -> Result<Vec<Arc<dyn SIMPAttachableAt<GuardLT>>>, CompilationError> {
        let path = String::from(ctx.path().as_ref().clone());
        self.paths.borrow_mut().push(path.clone());
        if self.metadata_error && path.contains("/#1/") {
            Err(CompilationError::TerminateWith(
                "second metadata failed".into(),
            ))
        } else {
            Ok(vec![])
        }
    }
}

impl Contract for Cached {
    declare! {finish, Self::owner, Self::owner}
    declare! {non updatable}
}

#[test]
fn custom_backend_caching_keeps_contextual_metadata_and_propagates_errors() {
    let contract = Cached::default();
    let first = contract.compile(context(false)).unwrap();
    assert_eq!(contract.calls.get(), 1);
    assert_eq!(
        *contract.paths.borrow(),
        [
            "custom/@finish_fn/#0/@metadata",
            "custom/@finish_fn/#1/@metadata"
        ]
    );
    assert_eq!(tree(&first).leaves().len(), 1);
    assert_eq!(contract.compile(context(false)).unwrap(), first);
    assert_eq!(contract.calls.get(), 2);
    let backend_error = Cached {
        backend_error: true,
        ..Default::default()
    };
    assert!(
        matches!(backend_error.compile(context(false)), Err(CompilationError::Policy(PolicyError::Backend(message))) if message == "translation rejected")
    );
    assert!(backend_error.paths.borrow().is_empty());
    let metadata_error = Cached {
        metadata_error: true,
        ..Default::default()
    };
    assert!(
        matches!(metadata_error.compile(context(false)), Err(CompilationError::TerminateWith(message)) if message == "second metadata failed")
    );
    assert_eq!(metadata_error.calls.get(), 1);
}

#[test]
fn raw_policy_fee_claims_require_a_real_satisfaction_weight_bound() {
    for emulated in [false, true] {
        assert!(matches!(
            Protected {
                require_feerate: true
            }
            .compile(context(emulated)),
            Err(CompilationError::UnknownSatisfactionWeight)
        ));
    }
}
