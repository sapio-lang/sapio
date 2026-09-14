use bitcoin::bip32::Xpriv;
use bitcoin::hashes::{sha256, Hash};
use bitcoin::psbt::{Input, Psbt};
use bitcoin::script::Builder;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::taproot::{LeafVersion, TapLeafHash, TaprootBuilder};
use bitcoin::{
    Amount, Network, OutPoint, PublicKey, ScriptBuf, TapSighashType, Transaction, TxIn, TxOut,
};
use miniscript::{Miniscript, Tap};
use sapio_base::util::CTVHash;
use sapio_psbt::annex;
use sapio_psbt::selected::{
    finalize_selected, HashRequirement, SatisfactionRecipe, SelectedError, SignatureSlot,
    StackElement,
};
use sapio_psbt::SigningKey;
use std::{collections::BTreeMap, str::FromStr};

fn roots() -> SigningKey {
    SigningKey(
        (1..=3)
            .map(|byte| Xpriv::new_master(Network::Regtest, &[byte; 32]).unwrap())
            .collect(),
    )
}

fn transaction(count: usize) -> Transaction {
    Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: (0..count)
            .map(|index| TxIn {
                previous_output: OutPoint {
                    vout: index as u32,
                    ..OutPoint::default()
                },
                ..Default::default()
            })
            .collect(),
        output: vec![TxOut {
            value: Amount::from_sat(900),
            script_pubkey: ScriptBuf::new(),
        }],
    }
}

fn tap_fixture(script: Option<ScriptBuf>, count: usize) -> (SigningKey, Psbt, ScriptBuf) {
    let keys = roots();
    let secp = Secp256k1::new();
    let public: Vec<_> = keys
        .0
        .iter()
        .map(|key| key.to_keypair(&secp).x_only_public_key().0)
        .collect();
    let script = script.unwrap_or_else(|| {
        Miniscript::<bitcoin::XOnlyPublicKey, Tap>::from_str(&format!(
            "or_i(pk({}),pk({}))",
            public[0], public[1]
        ))
        .unwrap()
        .encode()
    });
    let alternate =
        Miniscript::<bitcoin::XOnlyPublicKey, Tap>::from_str(&format!("pk({})", public[2]))
            .unwrap()
            .encode();
    let spend = TaprootBuilder::new()
        .add_leaf(1, script.clone())
        .unwrap()
        .add_leaf(1, alternate.clone())
        .unwrap()
        .finalize(&secp, public[2])
        .unwrap();
    let mut psbt = Psbt::from_unsigned_tx(transaction(count)).unwrap();
    for input in &mut psbt.inputs {
        input.witness_utxo = Some(TxOut {
            value: Amount::from_sat(1000),
            script_pubkey: ScriptBuf::new_p2tr_tweaked(spend.output_key()),
        });
        input.tap_internal_key = Some(public[2]);
        input.tap_merkle_root = spend.merkle_root();
        for script in [&script, &alternate] {
            input.tap_scripts.insert(
                spend
                    .control_block(&(script.clone(), LeafVersion::TapScript))
                    .unwrap(),
                (script.clone(), LeafVersion::TapScript),
            );
        }
    }
    (keys, psbt, script)
}

fn leaf_recipe(
    psbt: &Psbt,
    script: &ScriptBuf,
    key: bitcoin::XOnlyPublicKey,
    selector: bool,
) -> SatisfactionRecipe {
    let control = psbt.inputs[0]
        .tap_scripts
        .iter()
        .find(|(_, (candidate, _))| candidate == script)
        .unwrap()
        .0;
    let mut witness = vec![StackElement::SchnorrSignature {
        key,
        leaf: Some(TapLeafHash::from_script(script, LeafVersion::TapScript)),
    }];
    if selector {
        witness.push(StackElement::Literal(vec![1]));
    }
    witness.extend([
        StackElement::Literal(script.to_bytes()),
        StackElement::Literal(control.serialize()),
    ]);
    if let Some(annex) = annex::get(&psbt.inputs[0]).unwrap() {
        witness.push(StackElement::Literal(annex.to_vec()));
    }
    SatisfactionRecipe {
        script_sig: vec![],
        witness,
    }
}

#[test]
fn native_signing_is_limited_to_one_key_leaf_and_input() {
    let secp = Secp256k1::new();
    let (keys, mut psbt, script) = tap_fixture(None, 2);
    let key = keys.0[0].to_keypair(&secp).x_only_public_key().0;
    let leaf = TapLeafHash::from_script(&script, LeafVersion::TapScript);
    let untouched = psbt.inputs[1].clone();
    assert_eq!(
        keys.sign_selected_input_mut(
            &mut psbt,
            &secp,
            0,
            &[SignatureSlot::Schnorr {
                key,
                leaf: Some(leaf)
            }]
        )
        .unwrap(),
        1
    );
    assert!(psbt.inputs[0].tap_key_sig.is_none());
    assert_eq!(
        psbt.inputs[0]
            .tap_script_sigs
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        [(key, leaf)]
    );
    assert_eq!(psbt.inputs[1], untouched);
    // Having every private key does not authorize any other slot, including
    // a Program key deliberately omitted by the policy planner.
    let before = psbt.clone();
    assert_eq!(
        keys.sign_selected_input_mut(&mut psbt, &secp, 0, &[])
            .unwrap(),
        0
    );
    assert_eq!(psbt, before);
}

#[test]
fn exact_completion_keeps_inner_or_and_leaf_despite_extra_signatures() {
    let secp = Secp256k1::new();
    let (keys, mut psbt, script) = tap_fixture(None, 1);
    let public: Vec<_> = keys
        .0
        .iter()
        .map(|key| key.to_keypair(&secp).x_only_public_key().0)
        .collect();
    let leaf = TapLeafHash::from_script(&script, LeafVersion::TapScript);
    let recipe = leaf_recipe(&psbt, &script, public[0], true);
    let slots = [
        SignatureSlot::Schnorr {
            key: public[0],
            leaf: Some(leaf),
        },
        SignatureSlot::Schnorr {
            key: public[1],
            leaf: Some(leaf),
        },
        SignatureSlot::Schnorr {
            key: public[2],
            leaf: None,
        },
    ];
    keys.sign_selected_input_mut(&mut psbt, &secp, 0, &slots)
        .unwrap();
    let chosen = psbt.inputs[0].tap_script_sigs[&(public[0], leaf)].to_vec();
    let recipes = BTreeMap::from([(0, recipe)]);
    finalize_selected(&mut psbt, &secp, &recipes).unwrap();
    let stack = psbt.inputs[0]
        .final_script_witness
        .as_ref()
        .unwrap()
        .to_vec();
    assert_eq!(stack[0], chosen);
    assert_eq!(stack[1], [1]);
    assert_eq!(stack[2], script.as_bytes());
    assert_eq!(stack.len(), 4);
    assert!(psbt.inputs[0].tap_script_sigs.is_empty());
    assert!(psbt.inputs[0].tap_key_sig.is_none());
    let complete = psbt.clone();
    finalize_selected(&mut psbt, &secp, &recipes).unwrap();
    assert_eq!(
        psbt, complete,
        "completion is idempotent after partial metadata is cleared"
    );
}

#[test]
fn missing_or_invalid_selected_assets_preserve_original_psbt() {
    let secp = Secp256k1::new();
    let (keys, mut psbt, script) = tap_fixture(None, 1);
    let key = keys.0[0].to_keypair(&secp).x_only_public_key().0;
    let recipe = leaf_recipe(&psbt, &script, key, true);
    let before = psbt.clone();
    assert!(matches!(
        finalize_selected(&mut psbt, &secp, &BTreeMap::from([(0, recipe.clone())])),
        Err(SelectedError::MissingAsset { .. })
    ));
    assert_eq!(psbt, before);
    let leaf = TapLeafHash::from_script(&script, LeafVersion::TapScript);
    keys.sign_selected_input_mut(
        &mut psbt,
        &secp,
        0,
        &[SignatureSlot::Schnorr {
            key,
            leaf: Some(leaf),
        }],
    )
    .unwrap();
    psbt.unsigned_tx.output[0].value = Amount::from_sat(899);
    let before = psbt.clone();
    assert!(finalize_selected(&mut psbt, &secp, &BTreeMap::from([(0, recipe)])).is_err());
    assert_eq!(psbt, before);
}

#[test]
fn signing_rejects_funding_proof_and_scope_errors_without_partial_mutation() {
    let secp = Secp256k1::new();
    let (keys, original, script) = tap_fixture(None, 1);
    let key = keys.0[0].to_keypair(&secp).x_only_public_key().0;
    let leaf = TapLeafHash::from_script(&script, LeafVersion::TapScript);
    let valid = SignatureSlot::Schnorr {
        key,
        leaf: Some(leaf),
    };
    let mut candidate = original.clone();
    let before = candidate.clone();
    let invalid = SignatureSlot::Schnorr { key, leaf: None };
    assert!(keys
        .sign_selected_input_mut(&mut candidate, &secp, 0, &[valid.clone(), invalid])
        .is_err());
    assert_eq!(
        candidate, before,
        "a later invalid slot cannot leak an earlier signature"
    );
    for mutate in [0, 1, 2] {
        let mut candidate = original.clone();
        match mutate {
            0 => candidate.inputs[0].tap_scripts.clear(),
            1 => candidate.inputs[0].non_witness_utxo = Some(transaction(1)),
            _ => {
                candidate.inputs[0].sighash_type =
                    Some(bitcoin::psbt::PsbtSighashType::from_u32(0xff))
            }
        }
        let before = candidate.clone();
        assert!(keys
            .sign_selected_input_mut(&mut candidate, &secp, 0, std::slice::from_ref(&valid))
            .is_err());
        assert_eq!(candidate, before);
    }
}

fn ecdsa_fixture(kind: usize) -> (SigningKey, Psbt, SatisfactionRecipe) {
    let keys = roots();
    let key = PublicKey::new(keys.0[0].to_keypair(&Secp256k1::new()).public_key());
    let pk_script = ScriptBuf::new_p2pk(&key);
    let wpkh = ScriptBuf::new_p2wpkh(&key.wpubkey_hash().unwrap());
    let wsh = pk_script.to_p2wsh();
    let (output, redeem, witness_script) = match kind {
        0 => (ScriptBuf::new_p2pkh(&key.pubkey_hash()), None, None),
        1 => (wpkh.clone(), None, None),
        2 => (wpkh.to_p2sh(), Some(wpkh), None),
        3 => (wsh.clone(), None, Some(pk_script.clone())),
        4 => (wsh.to_p2sh(), Some(wsh), Some(pk_script.clone())),
        _ => unreachable!(),
    };
    let mut previous = transaction(1);
    previous.output[0] = TxOut {
        value: Amount::from_sat(1000),
        script_pubkey: output,
    };
    let mut tx = transaction(1);
    tx.input[0].previous_output = OutPoint::new(previous.compute_txid(), 0);
    let mut psbt = Psbt::from_unsigned_tx(tx).unwrap();
    psbt.inputs[0] = Input {
        witness_utxo: Some(previous.output[0].clone()),
        non_witness_utxo: (kind == 0).then_some(previous),
        redeem_script: redeem.clone(),
        witness_script,
        sighash_type: Some(bitcoin::EcdsaSighashType::AllPlusAnyoneCanPay.into()),
        ..Default::default()
    };
    let satisfaction = vec![
        StackElement::EcdsaSignature(key),
        StackElement::Literal(key.to_bytes()),
    ];
    let recipe = if kind == 0 {
        SatisfactionRecipe {
            script_sig: satisfaction,
            witness: vec![],
        }
    } else {
        SatisfactionRecipe {
            script_sig: redeem
                .map(|script| vec![StackElement::Literal(script.into_bytes())])
                .unwrap_or_default(),
            witness: if kind <= 2 {
                satisfaction
            } else {
                vec![
                    StackElement::EcdsaSignature(key),
                    StackElement::Literal(pk_script.into_bytes()),
                ]
            },
        }
    };
    (keys, psbt, recipe)
}

#[test]
fn selected_ecdsa_signing_and_completion_support_legacy_native_and_nested_segwit() {
    let secp = Secp256k1::new();
    for kind in 0..5 {
        let (keys, mut psbt, recipe) = ecdsa_fixture(kind);
        let key = PublicKey::new(keys.0[0].to_keypair(&secp).public_key());
        assert_eq!(
            keys.sign_selected_input_mut(&mut psbt, &secp, 0, &[SignatureSlot::Ecdsa(key)])
                .unwrap(),
            1
        );
        assert_eq!(
            psbt.inputs[0].partial_sigs[&key].sighash_type,
            bitcoin::EcdsaSighashType::AllPlusAnyoneCanPay
        );
        finalize_selected(&mut psbt, &secp, &BTreeMap::from([(0, recipe)])).unwrap();
        assert_eq!(psbt.inputs[0].final_script_witness.is_some(), kind != 0);
        assert_eq!(
            psbt.inputs[0].final_script_sig.is_some(),
            matches!(kind, 0 | 2 | 4)
        );
    }
    let (keys, mut legacy, _) = ecdsa_fixture(0);
    let key = PublicKey::new(keys.0[0].to_keypair(&secp).public_key());
    legacy.inputs[0].non_witness_utxo = None;
    let before = legacy.clone();
    assert!(keys
        .sign_selected_input_mut(&mut legacy, &secp, 0, &[SignatureSlot::Ecdsa(key)])
        .is_err());
    assert_eq!(legacy, before);
}

#[test]
fn selected_annex_is_exact_for_both_schnorr_signature_lengths() {
    let secp = Secp256k1::new();
    for flag in [TapSighashType::Default, TapSighashType::All] {
        let (keys, mut psbt, script) = tap_fixture(None, 1);
        annex::set(&mut psbt.inputs[0], Some(vec![0x50, 1, 2])).unwrap();
        psbt.inputs[0].sighash_type = Some(flag.into());
        let key = keys.0[0].to_keypair(&secp).x_only_public_key().0;
        let recipe = leaf_recipe(&psbt, &script, key, true);
        let leaf = TapLeafHash::from_script(&script, LeafVersion::TapScript);
        keys.sign_selected_input_mut(
            &mut psbt,
            &secp,
            0,
            &[SignatureSlot::Schnorr {
                key,
                leaf: Some(leaf),
            }],
        )
        .unwrap();
        let mut changed = psbt.clone();
        annex::set(&mut changed.inputs[0], Some(vec![0x50, 9])).unwrap();
        let before = changed.clone();
        assert!(
            finalize_selected(&mut changed, &secp, &BTreeMap::from([(0, recipe.clone())])).is_err()
        );
        assert_eq!(changed, before);
        finalize_selected(&mut psbt, &secp, &BTreeMap::from([(0, recipe)])).unwrap();
        let stack = psbt.inputs[0]
            .final_script_witness
            .as_ref()
            .unwrap()
            .to_vec();
        assert_eq!(
            stack[0].len(),
            if flag == TapSighashType::Default {
                64
            } else {
                65
            }
        );
        assert_eq!(stack.last().unwrap(), &[0x50, 1, 2]);
    }
}

#[test]
fn selected_preimages_are_checked_before_publishing_final_fields() {
    let secp = Secp256k1::new();
    let preimage = [42; 32];
    let digest = sha256::Hash::hash(&preimage);
    let key = roots().0[0].to_keypair(&secp).x_only_public_key().0;
    let script = Miniscript::<bitcoin::XOnlyPublicKey, Tap>::from_str(&format!(
        "and_v(v:sha256({digest}),pk({key}))"
    ))
    .unwrap()
    .encode();
    let (keys, mut psbt, script) = tap_fixture(Some(script), 1);
    let leaf = TapLeafHash::from_script(&script, LeafVersion::TapScript);
    let mut recipe = leaf_recipe(&psbt, &script, key, false);
    recipe
        .witness
        .insert(1, StackElement::Preimage(HashRequirement::Sha256(digest)));
    keys.sign_selected_input_mut(
        &mut psbt,
        &secp,
        0,
        &[SignatureSlot::Schnorr {
            key,
            leaf: Some(leaf),
        }],
    )
    .unwrap();
    psbt.inputs[0].sha256_preimages.insert(digest, vec![42; 31]);
    let before = psbt.clone();
    assert!(finalize_selected(&mut psbt, &secp, &BTreeMap::from([(0, recipe.clone())])).is_err());
    assert_eq!(psbt, before);
    psbt.inputs[0]
        .sha256_preimages
        .insert(digest, preimage.to_vec());
    finalize_selected(&mut psbt, &secp, &BTreeMap::from([(0, recipe)])).unwrap();
    assert_eq!(
        psbt.inputs[0]
            .final_script_witness
            .as_ref()
            .unwrap()
            .to_vec()[1],
        preimage
    );
}

#[test]
fn sponsors_settle_scriptsigs_before_selected_ctv_is_checked() {
    let secp = Secp256k1::new();
    for use_annex in [false, true] {
        let (keys, mut psbt, _) = tap_fixture(None, 2);
        let redeem = Builder::new().push_int(1).into_script();
        let script_sig = Builder::new()
            .push_slice(bitcoin::script::PushBytesBuf::try_from(redeem.to_bytes()).unwrap())
            .into_script();
        psbt.inputs[1] = Input {
            witness_utxo: Some(TxOut {
                value: Amount::from_sat(1000),
                script_pubkey: redeem.to_p2sh(),
            }),
            redeem_script: Some(redeem),
            ..Default::default()
        };
        let mut final_tx = psbt.unsigned_tx.clone();
        final_tx.input[1].script_sig = script_sig.clone();
        let key = keys.0[0].to_keypair(&secp).x_only_public_key().0;
        let script = Miniscript::<bitcoin::XOnlyPublicKey, Tap>::from_str(&format!(
            "and_v(txtmpl({}),pk({key}))",
            final_tx.get_ctv_hash(0)
        ))
        .unwrap()
        .encode();
        let (_, replacement, _) = tap_fixture(Some(script.clone()), 2);
        psbt.inputs[0] = replacement.inputs[0].clone();
        if use_annex {
            annex::set(&mut psbt.inputs[0], Some(vec![0x50, 7])).unwrap();
        }
        let recipe = leaf_recipe(&psbt, &script, key, false);
        let leaf = TapLeafHash::from_script(&script, LeafVersion::TapScript);
        keys.sign_selected_input_mut(
            &mut psbt,
            &secp,
            0,
            &[SignatureSlot::Schnorr {
                key,
                leaf: Some(leaf),
            }],
        )
        .unwrap();
        let mut wrong = psbt.clone();
        wrong.unsigned_tx.output[0].value = Amount::from_sat(899);
        let before = wrong.clone();
        assert!(
            finalize_selected(&mut wrong, &secp, &BTreeMap::from([(0, recipe.clone())])).is_err()
        );
        assert_eq!(wrong, before);
        finalize_selected(&mut psbt, &secp, &BTreeMap::from([(0, recipe)])).unwrap();
        assert_eq!(psbt.inputs[1].final_script_sig, Some(script_sig));
    }
}

#[test]
fn retrying_selected_signing_preserves_verified_existing_contributions() {
    let secp = Secp256k1::new();
    let (keys, mut psbt, script) = tap_fixture(None, 1);
    let key = keys.0[0].to_keypair(&secp).x_only_public_key().0;
    let leaf = TapLeafHash::from_script(&script, LeafVersion::TapScript);
    let slot = SignatureSlot::Schnorr {
        key,
        leaf: Some(leaf),
    };
    psbt.inputs[0].sighash_type = Some(TapSighashType::All.into());
    keys.sign_selected_input_mut(&mut psbt, &secp, 0, std::slice::from_ref(&slot))
        .unwrap();
    psbt.inputs[0].sighash_type = None;
    let before = psbt.clone();
    assert_eq!(
        keys.sign_selected_input_mut(&mut psbt, &secp, 0, std::slice::from_ref(&slot))
            .unwrap(),
        0
    );
    assert_eq!(
        psbt, before,
        "an existing ALL signature is preserved without a declared flag"
    );
    psbt.unsigned_tx.output[0].value = Amount::from_sat(899);
    let before = psbt.clone();
    assert!(keys
        .sign_selected_input_mut(&mut psbt, &secp, 0, &[slot])
        .is_err());
    assert_eq!(
        psbt, before,
        "invalid old signatures cannot be silently repaired"
    );

    let (keys, mut psbt, _) = ecdsa_fixture(1);
    let key = PublicKey::new(keys.0[0].to_keypair(&secp).public_key());
    let slot = SignatureSlot::Ecdsa(key);
    keys.sign_selected_input_mut(&mut psbt, &secp, 0, std::slice::from_ref(&slot))
        .unwrap();
    psbt.inputs[0].sighash_type = None;
    let before = psbt.clone();
    assert_eq!(
        keys.sign_selected_input_mut(&mut psbt, &secp, 0, std::slice::from_ref(&slot))
            .unwrap(),
        0
    );
    assert_eq!(psbt, before);
    psbt.unsigned_tx.output[0].value = Amount::from_sat(899);
    let before = psbt.clone();
    assert!(keys
        .sign_selected_input_mut(&mut psbt, &secp, 0, &[slot])
        .is_err());
    assert_eq!(psbt, before);
}

#[test]
fn selected_keypath_completion_uses_the_validated_internal_key_tweak() {
    let secp = Secp256k1::new();
    let (keys, mut psbt, _) = tap_fixture(None, 1);
    let key = psbt.inputs[0].tap_internal_key.unwrap();
    let recipe = SatisfactionRecipe {
        script_sig: vec![],
        witness: vec![StackElement::SchnorrSignature { key, leaf: None }],
    };
    let slot = SignatureSlot::Schnorr { key, leaf: None };
    keys.sign_selected_input_mut(&mut psbt, &secp, 0, std::slice::from_ref(&slot))
        .unwrap();
    let before = psbt.clone();
    assert_eq!(
        keys.sign_selected_input_mut(&mut psbt, &secp, 0, &[slot])
            .unwrap(),
        0
    );
    assert_eq!(psbt, before);
    finalize_selected(&mut psbt, &secp, &BTreeMap::from([(0, recipe)])).unwrap();
    assert_eq!(
        psbt.inputs[0].final_script_witness.as_ref().unwrap().len(),
        1
    );
}

#[test]
fn empty_bare_satisfaction_retains_its_only_completion_marker() {
    let secp = Secp256k1::new();
    // This uses the finalizer's existing interpreter support; it does not
    // extend the set of standard Bare descriptors accepted by the planner.
    let mut psbt = Psbt::from_unsigned_tx(transaction(1)).unwrap();
    psbt.inputs[0].witness_utxo = Some(TxOut {
        value: Amount::from_sat(1000),
        script_pubkey: Builder::new().push_int(1).into_script(),
    });
    let recipes = BTreeMap::from([(
        0,
        SatisfactionRecipe {
            script_sig: vec![],
            witness: vec![],
        },
    )]);
    finalize_selected(&mut psbt, &secp, &recipes).unwrap();
    assert_eq!(psbt.inputs[0].final_script_sig, Some(ScriptBuf::new()));
    assert!(psbt.inputs[0].final_script_witness.is_none());
    let restored = Psbt::deserialize(&psbt.serialize()).unwrap();
    assert_eq!(restored, psbt);
    finalize_selected(&mut psbt, &secp, &recipes).unwrap();
    assert_eq!(psbt, restored);
}
