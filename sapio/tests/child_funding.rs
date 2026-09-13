use bitcoin::hashes::sha256;
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::{Secp256k1, SecretKey};
use bitcoin::{Address, Amount, Network, OutPoint, Transaction, TxIn, TxOut, Txid};
use sapio::contract::abi::object::{ArtifactErrorKind, ObjectError};
use sapio::contract::{Compilable, CompilationError, Compiled, Context, Contract};
use sapio::{continuation, declare, guard, then};
use sapio_base::covenant::LoweringPlan;
use sapio_base::txindex::{TxIndex, TxIndexError};
use sapio_base::Clause;
use sapio_ctv_emulator_trait::{CTVEmulator, EmulatorError};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;

fn context(funds: u64, path: &str) -> Context {
    Context::new(
        Network::Regtest,
        Amount::from_sat(funds),
        LoweringPlan::Native,
        path.try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    )
}

fn address() -> Compiled {
    Compiled::from_address(
        Address::from_str("bcrt1qumrrqgt7e3a7damzm8x97m6sjs20u8hjw2hcjj")
            .unwrap()
            .require_network(Network::Regtest)
            .unwrap(),
        Amount::ZERO,
    )
}

struct Payment {
    destination: Compiled,
    amount: u64,
    external: u64,
    fees: u64,
}

impl Payment {
    #[then]
    fn pay(self, ctx: Context) {
        let mut builder = ctx.template();
        if self.external != 0 {
            builder = builder
                .add_sequence()
                .add_amount(Amount::from_sat(self.external))?;
        }
        builder
            .add_output(Amount::from_sat(self.amount), &self.destination, None)?
            .add_fees(Amount::from_sat(self.fees))?
            .into()
    }
}

impl Contract for Payment {
    declare! {actions, Self::pay}
}

fn payment(destination: Compiled, amount: u64, external: u64, fees: u64, path: &str) -> Compiled {
    Payment {
        destination,
        amount,
        external,
        fees,
    }
    .compile(context(amount + fees - external, path))
    .unwrap()
}

struct Alternatives;

impl Alternatives {
    #[guard]
    fn signed(self, _ctx: Context) {
        Clause::Key(
            SecretKey::from_slice(&[1; 32])
                .unwrap()
                .keypair(&Secp256k1::new())
                .x_only_public_key()
                .0,
        )
    }

    #[then]
    fn pay(self, ctx: Context) {
        ctx.template()
            .add_sequence()
            .add_amount(Amount::from_sat(700))?
            .add_output(Amount::from_sat(1_000), &address(), None)?
            .add_fees(Amount::from_sat(100))?
            .into()
    }

    #[continuation(guarded_by = "[Self::signed]", default)]
    fn suggest(self, ctx: Context, _args: ()) {
        ctx.template()
            .add_sequence()
            .add_amount(Amount::from_sat(700))?
            .add_output(Amount::from_sat(1_100), &address(), None)?
            .add_fees(Amount::from_sat(200))?
            .into()
    }
}

impl Contract for Alternatives {
    declare! {actions, Self::pay, Self::suggest}
}

#[test]
fn compiler_retains_the_largest_contract_contribution_across_both_template_maps() {
    let child = Alternatives.compile(context(600, "child")).unwrap();
    child.validate().unwrap();
    assert_eq!(child.required_input_amount, Amount::from_sat(600));
    assert_eq!(
        child
            .ctv_to_tx
            .values()
            .next()
            .unwrap()
            .required_input_amount,
        Amount::from_sat(400)
    );
    assert_eq!(
        child
            .suggested_txs
            .values()
            .next()
            .unwrap()
            .required_input_amount,
        Amount::from_sat(600)
    );
    let mut json = serde_json::to_value(&child).unwrap();
    assert_eq!(json["required_input_amount_sats"], 600);
    assert!(json.get("amount_range").is_none());
    json.as_object_mut()
        .unwrap()
        .remove("required_input_amount_sats");
    assert!(serde_json::from_value::<Compiled>(json).is_err());
}

#[test]
fn child_output_covers_its_contract_contribution_without_replacing_auxiliary_inputs() {
    for suggested in [false, true] {
        let mut child = payment(address(), 1_000, 700, 100, "child");
        assert_eq!(child.required_input_amount, Amount::from_sat(400));
        assert_eq!(
            child.ctv_to_tx.values().next().unwrap().max,
            Amount::from_sat(1_100)
        );
        if suggested {
            child.suggested_txs = std::mem::take(&mut child.ctv_to_tx);
        }
        let parent = payment(child, 400, 0, 0, "parent");
        let decoded: Compiled =
            serde_json::from_slice(&serde_json::to_vec(&parent).unwrap()).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded.required_input_amount, Amount::from_sat(400));
    }
}

#[test]
fn unrestricted_address_has_no_minimum_output_amount() {
    let destination = address();
    assert_eq!(destination.required_input_amount, Amount::ZERO);
    let parent = payment(destination, 1, 0, 0, "parent");
    parent.validate().unwrap();
    assert_eq!(parent.required_input_amount, Amount::ONE_SAT);
}

struct MinimumBalance;

impl MinimumBalance {
    #[guard]
    fn signed(self, _ctx: Context) {
        Clause::Key(
            SecretKey::from_slice(&[1; 32])
                .unwrap()
                .keypair(&Secp256k1::new())
                .x_only_public_key()
                .0,
        )
    }
}

impl Contract for MinimumBalance {
    declare! {finish, Self::signed}

    fn ensure_amount(&self, _ctx: Context) -> Result<Amount, CompilationError> {
        Ok(Amount::from_sat(1_000))
    }
}

#[test]
fn ensure_amount_only_floor_survives_compilation_and_reused_child_outputs() {
    let child = MinimumBalance.compile(context(1_000, "child")).unwrap();
    assert!(child.ctv_to_tx.is_empty());
    assert!(child.suggested_txs.is_empty());
    assert_eq!(child.required_input_amount, Amount::from_sat(1_000));
    let child: Compiled = serde_json::from_slice(&serde_json::to_vec(&child).unwrap()).unwrap();
    child.validate().unwrap();
    for contract in [&child as &dyn Compilable, &MinimumBalance] {
        let result =
            context(999, "parent")
                .template()
                .add_output(Amount::from_sat(999), contract, None);
        assert!(matches!(
            result,
            Err(CompilationError::UnderfundedOutput { available, required, .. })
                if available == Amount::from_sat(999) && required == Amount::from_sat(1_000)
        ));
    }
    payment(child, 1_000, 0, 0, "parent").validate().unwrap();
}

struct MutatedChild;

impl MutatedChild {
    #[then]
    fn pay(self, ctx: Context) {
        let mut template: sapio::template::Template = ctx
            .template()
            .add_output(Amount::from_sat(1_000), &address(), None)?
            .into();
        // Public template fields let an action invalidate a previously checked
        // output. Fresh compilation must validate the completed artifact too.
        template.outputs[0].contract.required_input_amount = Amount::from_sat(1_001);
        Ok(Box::new(std::iter::once(Ok(template))))
    }
}

impl Contract for MutatedChild {
    declare! {actions, Self::pay}
}

#[test]
fn fresh_actions_cannot_return_an_inconsistent_child_graph() {
    let error = MutatedChild.compile(context(1_000, "mutated")).unwrap_err();
    let CompilationError::InvalidArtifact(error) = error else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(
        error.kind,
        ArtifactErrorKind::UnderfundedChild {
            index: 0,
            available: Amount::from_sat(1_000),
            required: Amount::from_sat(1_001),
        }
    );
    assert!(matches!(
        context(1_000, "parent").template().add_output(
            Amount::from_sat(1_000),
            &MutatedChild,
            None
        ),
        Err(CompilationError::InvalidArtifact(_))
    ));
}

#[test]
fn contract_minimum_round_trips_as_lossless_integer_satoshis() {
    for sats in [(1u64 << 53) + 1, u64::MAX] {
        let mut object = address();
        object.required_input_amount = Amount::from_sat(sats);
        let encoded = serde_json::to_vec(&object).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(json["required_input_amount_sats"].as_u64(), Some(sats));
        let decoded: Compiled = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.required_input_amount, Amount::from_sat(sats));
        decoded.validate().unwrap();
    }
}

struct KnownFunding(Arc<Transaction>);

impl TxIndex for KnownFunding {
    fn lookup_tx(&self, txid: &Txid) -> Result<Arc<Transaction>, TxIndexError> {
        assert_eq!(*txid, self.0.compute_txid());
        Ok(self.0.clone())
    }

    fn add_tx(&self, _: Arc<Transaction>) -> Result<Txid, TxIndexError> {
        panic!("finish-only contract has no template transaction to insert");
    }
}

#[test]
fn known_funding_obeys_ensure_amount_even_without_any_templates() {
    let object = MinimumBalance.compile(context(1_000, "minimum")).unwrap();
    for amount in [999, 1_000] {
        let funding = Arc::new(Transaction {
            version: bitcoin::transaction::Version(2),
            lock_time: bitcoin::absolute::LockTime::from_consensus(0),
            input: vec![TxIn::default()],
            output: vec![TxOut {
                value: Amount::from_sat(amount),
                script_pubkey: object.address.clone().into(),
            }],
        });
        let outpoint = OutPoint::new(funding.compute_txid(), 0);
        let result = object.bind_psbt(
            outpoint,
            BTreeMap::new(),
            Rc::new(KnownFunding(funding)),
            &NoEffects,
        );
        if amount == 999 {
            assert!(matches!(
                result,
                Err(ObjectError::InvalidFunding { outpoint: actual, .. }) if actual == outpoint
            ));
        } else {
            result.unwrap();
        }
    }
}

#[test]
fn object_minimum_cannot_understate_committed_or_suggested_requirements() {
    for suggested in [false, true] {
        let mut child = payment(address(), 1_000, 700, 100, "child");
        let hash = *child.ctv_to_tx.keys().next().unwrap();
        if suggested {
            child.suggested_txs = std::mem::take(&mut child.ctv_to_tx);
        }
        child.required_input_amount = Amount::from_sat(399);
        let child: Compiled = serde_json::from_slice(&serde_json::to_vec(&child).unwrap()).unwrap();
        let error = child.validate().unwrap_err();
        assert_eq!(error.path, child.root_path);
        assert_eq!(error.template, Some(hash));
        assert_eq!(error.kind, ArtifactErrorKind::InvalidInputRequirement);
    }
}

struct NoEffects;

impl TxIndex for NoEffects {
    fn lookup_tx(&self, _: &Txid) -> Result<Arc<Transaction>, TxIndexError> {
        panic!("underfunded child reached funding lookup");
    }

    fn add_tx(&self, _: Arc<Transaction>) -> Result<Txid, TxIndexError> {
        panic!("underfunded child reached transaction insertion");
    }
}

impl CTVEmulator for NoEffects {
    fn get_signer_for(&self, _: sha256::Hash) -> Result<Clause, EmulatorError> {
        panic!("underfunded child requested a signer");
    }

    fn sign(&self, _: Psbt) -> Result<Psbt, EmulatorError> {
        panic!("underfunded child reached signing");
    }
}

#[test]
fn serialized_underfunded_children_fail_before_any_funding_or_signing_effect() {
    for suggested in [false, true] {
        let child = payment(address(), 1_000, 0, 0, "child");
        let mut parent = payment(child, 1_000, 0, 0, "parent");
        let hash = *parent.ctv_to_tx.keys().next().unwrap();
        let child = &mut parent.ctv_to_tx.get_mut(&hash).unwrap().outputs[0].contract;
        let template = child.ctv_to_tx.values_mut().next().unwrap();
        // Reserved fees are not part of the CTV commitment. Increasing the
        // child's requirement preserves its transaction but makes this parent
        // output insufficient. All cached transaction hashes still agree.
        template.max = Amount::from_sat(1_100);
        template.required_input_amount = Amount::from_sat(1_100);
        child.required_input_amount = Amount::from_sat(1_100);
        if suggested {
            child.suggested_txs = std::mem::take(&mut child.ctv_to_tx);
        }
        child.validate().unwrap();
        let parent: Compiled =
            serde_json::from_slice(&serde_json::to_vec(&parent).unwrap()).unwrap();
        let expected = ArtifactErrorKind::UnderfundedChild {
            index: 0,
            available: Amount::from_sat(1_000),
            required: Amount::from_sat(1_100),
        };
        let error = parent.validate().unwrap_err();
        assert_eq!(error.path, parent.root_path);
        assert_eq!(error.template, Some(hash));
        assert_eq!(error.kind, expected);
        let error = parent
            .bind_psbt(
                OutPoint::default(),
                BTreeMap::new(),
                Rc::new(NoEffects),
                &NoEffects,
            )
            .unwrap_err();
        let ObjectError::InvalidArtifact(error) = error else {
            panic!("unexpected error: {error}");
        };
        assert_eq!(error.kind, expected);
    }
}
