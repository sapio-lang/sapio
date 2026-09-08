use super::*;
use bitcoin::secp256k1::{Keypair, Message, Secp256k1, SecretKey};
use bitcoin::util::psbt::PartiallySignedTransaction as Psbt;
use bitcoin::util::sighash::{Prevouts, SighashCache};
use bitcoin::util::taproot::TapLeafHash;
use bitcoin::{Network, OutPoint, SchnorrSig, SchnorrSighashType, Transaction, TxOut};
use sapio::contract::abi::object::SupportedDescriptors;
use sapio::contract::abi::studio::SapioStudioFormat;
use sapio::contract::Compilable;
use sapio_base::effects::EffectPath;
use sapio_base::miniscript::ord::{envelope::Envelope, Inscription};
use sapio_base::miniscript::psbt::PsbtExt;
use sapio_base::miniscript::Descriptor;
use sapio_base::plugin_args::OrdinalsInfo;
use sapio_base::txindex::{TxIndex, TxIndexLogger};
use sapio_ctv_emulator_trait::CTVAvailable;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;

fn keypair(byte: u8) -> Keypair {
    Keypair::from_secret_key(
        &Secp256k1::new(),
        &SecretKey::from_slice(&[byte; 32]).unwrap(),
    )
}

fn contract() -> InscribingStep {
    InscribingStep {
        owner: keypair(1).x_only_public_key().0,
        data: vec![0x5a; 521],
        content_type: "application/octet-stream".into(),
    }
}

fn context(ranges: &[(u64, u64)]) -> Context {
    Context::new(
        Network::Regtest,
        Amount::from_sat(10_000),
        Arc::new(CTVAvailable),
        EffectPath::try_from("inscription").unwrap(),
        Arc::new(Default::default()),
        Some(OrdinalsInfo(
            ranges
                .iter()
                .map(|(start, end)| (Ordinal(*start), Ordinal(*end)))
                .collect(),
        )),
    )
}

#[test]
fn malformed_ordinal_ranges_return_errors() {
    let contract = contract();
    for ranges in [
        vec![],
        vec![(2, 1)],
        vec![(1, 1)],
        vec![(0, u64::MAX), (0, 1)],
        vec![(0, 9_999)],
        vec![(0, 10_001)],
        vec![(0, 5_000), (0, 5_000)],
    ] {
        assert!(matches!(
            contract.ensure_amount(context(&ranges)),
            Err(CompilationError::OrdinalsError(_))
        ));
        assert!(matches!(
            contract.continue_reveal(context(&ranges), Reveal::default()),
            Err(CompilationError::OrdinalsError(_))
        ));
    }
}

#[test]
fn reveal_fee_preserves_the_inscribed_sat_and_padding() {
    let contract = contract();
    for fee in [9_500, 10_000, 10_001, u64::MAX] {
        assert!(matches!(
            contract.continue_reveal(
                context(&[(0, 10_000)]),
                Reveal {
                    fee: fee.into(),
                    alternative: None,
                },
            ),
            Err(CompilationError::OutOfFunds)
        ));
    }
    let template = contract
        .continue_reveal(
            context(&[(0, 10_000)]),
            Reveal {
                fee: 9_499.into(),
                alternative: None,
            },
        )
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(template.tx.output[0].value, 501);
}

#[test]
fn oversized_content_type_is_rejected_before_funding() {
    let mut contract = contract();
    contract.content_type = "a".repeat(521);
    assert!(contract.compile(context(&[(0, 10_000)])).is_err());
}

#[test]
fn owner_reveal_preserves_inscription_through_artifact_and_psbt_round_trips() {
    let secp = Secp256k1::new();
    let contract = contract();
    let compiled = contract.compile(context(&[(0, 10_000)])).unwrap();
    let json = serde_json::to_value(&compiled).unwrap();
    let compiled: Compiled = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(serde_json::to_value(&compiled).unwrap(), json);
    compiled.validate().unwrap();
    let Some(SupportedDescriptors::XOnly(Descriptor::Tr(descriptor))) = &compiled.descriptor else {
        panic!("expected a Taproot inscription descriptor");
    };
    assert_ne!(*descriptor.internal_key(), contract.owner);

    let funding = Transaction {
        version: 2,
        lock_time: 0,
        input: vec![bitcoin::TxIn::default()],
        output: vec![TxOut {
            value: 10_000,
            script_pubkey: compiled.address.clone().into(),
        }],
    };
    let prevouts = funding.output.clone();
    let txindex: Rc<dyn TxIndex> = Rc::new(TxIndexLogger::new());
    let txid = txindex.add_tx(Arc::new(funding)).unwrap();
    let bound = compiled
        .bind_psbt(
            OutPoint::new(txid, 0),
            BTreeMap::new(),
            txindex,
            &CTVAvailable,
        )
        .unwrap();
    let transactions: Vec<_> = bound
        .program
        .values()
        .flat_map(|object| &object.txs)
        .collect();
    assert_eq!(transactions.len(), 1);
    let SapioStudioFormat::LinkedPSBT { psbt, .. } = transactions[0];
    let mut psbt = Psbt::from_str(psbt).unwrap();
    assert_eq!(psbt.inputs[0].tap_scripts.len(), 1);
    let (script, leaf_version) = psbt.inputs[0].tap_scripts.values().next().unwrap();
    let leaf_hash = TapLeafHash::from_script(script, *leaf_version);
    let sighash = SighashCache::new(&psbt.unsigned_tx)
        .taproot_script_spend_signature_hash(
            0,
            &Prevouts::All(&prevouts),
            leaf_hash,
            SchnorrSighashType::Default,
        )
        .unwrap();
    let message = Message::from_digest_slice(&sighash[..]).unwrap();
    assert!(psbt.clone().finalize_mut(&secp).is_err());

    let signature = |key| SchnorrSig {
        sig: secp.sign_schnorr_no_aux_rand(&message, &keypair(key)),
        hash_ty: SchnorrSighashType::Default,
    };
    let mut wrong_owner = psbt.clone();
    wrong_owner.inputs[0]
        .tap_script_sigs
        .insert((contract.owner, leaf_hash), signature(2));
    assert!(wrong_owner.finalize_mut(&secp).is_err());
    let owner_signature = signature(1);
    secp.verify_schnorr(&owner_signature.sig, &message, &contract.owner)
        .unwrap();
    psbt.inputs[0]
        .tap_script_sigs
        .insert((contract.owner, leaf_hash), owner_signature);
    psbt.finalize_mut(&secp).unwrap();
    let revealed = psbt.extract(&secp).unwrap();
    assert_eq!(revealed.output[0].value, 9_500);
    let envelopes = Envelope::<Inscription>::from_transaction(&revealed);
    assert_eq!(envelopes.len(), 1);
    assert_eq!(envelopes[0].payload.body.as_ref(), Some(&contract.data));
    assert_eq!(
        envelopes[0].payload.content_type.as_deref(),
        Some(contract.content_type.as_bytes())
    );
}
