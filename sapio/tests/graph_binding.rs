use bitcoin::consensus::deserialize;
use bitcoin::psbt::PartiallySignedTransaction as Psbt;
use bitcoin::{Address, Amount, Network, OutPoint};
use sapio::contract::abi::continuation::ContinuationPoint;
use sapio::contract::abi::studio::{Program, SapioStudioFormat};
use sapio::contract::{Compilable, Compiled, Context, Contract};
use sapio::{declare, then};
use sapio_base::serialization_helpers::SArc;
use sapio_base::txindex::TxIndexLogger;
use sapio_ctv_emulator_trait::CTVAvailable;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;

struct Payment(Vec<Compiled>);

impl Payment {
    #[then]
    fn pay(self, ctx: Context) {
        let mut builder = ctx.template();
        for destination in &self.0 {
            builder = builder.add_output(Amount::from_sat(1_000), destination, None)?;
        }
        builder.into()
    }
}

impl Contract for Payment {
    declare! {then, Self::pay}
    declare! {non updatable}
}

fn payment(destinations: Vec<Compiled>, path: &str) -> Compiled {
    let amount = Amount::from_sat(1_000 * destinations.len() as u64);
    Payment(destinations)
        .compile(Context::new(
            Network::Regtest,
            amount,
            Arc::new(CTVAvailable),
            path.try_into().unwrap(),
            Arc::new(Default::default()),
            None,
        ))
        .unwrap()
}

fn leaf(label: &str) -> Compiled {
    let mut object = Compiled::from_address(
        Address::from_str("bcrt1qumrrqgt7e3a7damzm8x97m6sjs20u8hjw2hcjj").unwrap(),
        None,
    );
    object.metadata.extra.insert("label".into(), label.into());
    object
}

fn bind(object: &Compiled) -> Program {
    object
        .bind_psbt(
            OutPoint::default(),
            BTreeMap::new(),
            Rc::new(TxIndexLogger::new()),
            &CTVAvailable,
        )
        .unwrap()
}

fn psbt(format: &SapioStudioFormat) -> Psbt {
    let SapioStudioFormat::LinkedPSBT { psbt, .. } = format;
    deserialize(&base64::decode(psbt).unwrap()).unwrap()
}

#[test]
fn reused_leaf_paths_preserve_every_output_and_its_metadata() {
    let object = payment(vec![leaf("alice"), leaf("bob")], "payment");
    let program = bind(&object);
    assert_eq!(program.program.len(), 3);
    let root = program.program.get(&object.root_path).unwrap();
    let tx = psbt(&root.txs[0]).unsigned_tx;
    for (vout, label) in [(0, "alice"), (1, "bob")] {
        let child = program
            .program
            .values()
            .find(|node| node.out == OutPoint::new(tx.txid(), vout))
            .unwrap();
        assert!(child.txs.is_empty());
        assert_eq!(child.metadata.extra["label"], label);
    }
    assert_eq!(
        serde_json::to_value(bind(&object)).unwrap(),
        serde_json::to_value(&program).unwrap()
    );
    let roundtrip: Program =
        serde_json::from_value(serde_json::to_value(&program).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(roundtrip).unwrap(),
        serde_json::to_value(&program).unwrap()
    );
}

#[test]
fn reused_contract_paths_preserve_both_bound_spending_branches() {
    let mut child = payment(vec![leaf("destination")], "reused");
    let continuation: Arc<sapio_base::effects::EffectPath> =
        Arc::new("reused/continue".try_into().unwrap());
    child.continue_apis.insert(
        SArc(continuation.clone()),
        ContinuationPoint::at(None, continuation),
    );
    let object = payment(vec![child.clone(), child.clone()], "parent");
    let program = bind(&object);
    assert_eq!(program.program.len(), 5);
    let parent = psbt(&program.program.get(&object.root_path).unwrap().txs[0]).unsigned_tx;
    let mut spent = BTreeSet::new();
    for node in program
        .program
        .values()
        .filter(|node| node.out.txid == parent.txid())
    {
        assert_eq!(node.txs.len(), 1);
        assert_eq!(node.source_path.as_ref(), Some(&child.root_path));
        assert_eq!(node.continue_apis, child.continue_apis);
        let bound = psbt(&node.txs[0]);
        assert_eq!(bound.unsigned_tx.input[0].previous_output, node.out);
        assert_eq!(bound.inputs[0].non_witness_utxo, Some(parent.clone()));
        spent.insert(node.out.vout);
    }
    assert_eq!(spent, BTreeSet::from([0, 1]));
}

#[test]
fn suggested_and_enforced_transitions_have_distinct_binding_paths() {
    let mut object = payment(vec![leaf("destination")], "parent");
    object.suggested_txs = object.ctv_to_tx.clone();
    let program = bind(&object);
    assert_eq!(program.program.len(), 3);
    let root = program.program.get(&object.root_path).unwrap();
    assert_eq!(root.txs.len(), 2);
    let leaves: Vec<_> = program
        .program
        .iter()
        .filter(|(_, node)| node.txs.is_empty())
        .collect();
    assert_eq!(leaves.len(), 2);
    assert_eq!(leaves[0].1.out, leaves[1].1.out);
    assert_ne!(leaves[0].0, leaves[1].0);
}
