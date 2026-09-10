use bitcoin::blockdata::opcodes::all::{OP_CHECKSIG, OP_CODESEPARATOR};
use bitcoin::blockdata::script::Builder;
use bitcoin::consensus::{deserialize, serialize};
use bitcoin::secp256k1::{Message, Secp256k1};
use bitcoin::util::bip32::{DerivationPath, ExtendedPrivKey};
use bitcoin::util::psbt::PartiallySignedTransaction as Psbt;
use bitcoin::util::sighash::{Annex, Prevouts, SighashCache};
use bitcoin::util::taproot::{LeafVersion, TapLeafHash, TaprootBuilder};
use bitcoin::{Network, OutPoint, SchnorrSighashType, Script, Transaction, TxIn, TxOut, Witness};
use sapio_base::util::CTVHash;
use sapio_psbt::annex::{self, AnnexError};
use sapio_psbt::finalize::finalize;
use sapio_psbt::SigningKey;
use std::str::FromStr;

fn fixture(script: Option<Script>, count: usize) -> (SigningKey, Psbt, Script) {
    let secp = Secp256k1::new();
    let root = ExtendedPrivKey::new_master(Network::Regtest, &[42; 32]).unwrap();
    let key = root.to_keypair(&secp).x_only_public_key().0;
    let script = script.unwrap_or_else(|| {
        Builder::new()
            .push_slice(&key.serialize())
            .push_opcode(OP_CHECKSIG)
            .into_script()
    });
    let leaf = TapLeafHash::from_script(&script, LeafVersion::TapScript);
    let spend = TaprootBuilder::new()
        .add_leaf(0, script.clone())
        .unwrap()
        .finalize(&secp, key)
        .unwrap();
    let prevout = TxOut {
        value: 10_000,
        script_pubkey: Script::new_v1_p2tr_tweaked(spend.output_key()),
    };
    let tx = Transaction {
        version: 2,
        lock_time: 0,
        input: (0..count)
            .map(|index| TxIn {
                previous_output: OutPoint {
                    vout: index as u32,
                    ..OutPoint::default()
                },
                ..TxIn::default()
            })
            .collect(),
        output: vec![TxOut {
            value: 9_000,
            script_pubkey: Script::new(),
        }],
    };
    let mut psbt = Psbt::from_unsigned_tx(tx).unwrap();
    for input in &mut psbt.inputs {
        input.witness_utxo = Some(prevout.clone());
        input.tap_internal_key = Some(key);
        input.tap_merkle_root = spend.merkle_root();
        input.tap_key_origins.insert(
            key,
            (
                vec![leaf],
                (root.fingerprint(&secp), DerivationPath::master()),
            ),
        );
        input.tap_scripts.insert(
            spend
                .control_block(&(script.clone(), LeafVersion::TapScript))
                .unwrap(),
            (script.clone(), LeafVersion::TapScript),
        );
        annex::set(input, Some(vec![0x50, 1, 2, 3])).unwrap();
    }
    (SigningKey(vec![root]), psbt, script)
}

fn signed(scriptpath: bool, hash_type: SchnorrSighashType) -> (Psbt, Script) {
    let (keys, mut psbt, script) = fixture(None, 1);
    keys.sign_psbt_mut(&mut psbt, &Secp256k1::new(), hash_type)
        .unwrap();
    if scriptpath {
        psbt.inputs[0].tap_key_sig = None;
    }
    (psbt, script)
}

#[test]
fn annex_extension_roundtrips_and_enforces_exact_bounds() {
    let (_, mut psbt, _) = fixture(None, 1);
    assert_eq!(deserialize::<Psbt>(&serialize(&psbt)).unwrap(), psbt);
    for bytes in [vec![], vec![0x51], vec![0; 20]] {
        assert_eq!(
            annex::set(&mut psbt.inputs[0], Some(bytes)),
            Err(AnnexError::InvalidPrefix)
        );
    }
    annex::set(
        &mut psbt.inputs[0],
        Some(vec![0x50; annex::MAX_ANNEX_BYTES]),
    )
    .unwrap();
    assert_eq!(
        annex::get(&psbt.inputs[0]).unwrap().unwrap().len(),
        annex::MAX_ANNEX_BYTES
    );
    assert_eq!(
        annex::set(
            &mut psbt.inputs[0],
            Some(vec![0x50; annex::MAX_ANNEX_BYTES + 1])
        ),
        Err(AnnexError::TooLarge(annex::MAX_ANNEX_BYTES + 1))
    );
    psbt.inputs[0].proprietary.insert(annex::field(), vec![]);
    assert!(sapio_psbt::validate_psbt(&psbt).is_err());
}

#[test]
fn both_signature_forms_bind_the_exact_annex_on_both_spend_paths() {
    let secp = Secp256k1::new();
    for hash_type in [SchnorrSighashType::Default, SchnorrSighashType::All] {
        let (psbt, script) = signed(false, hash_type);
        let input = &psbt.inputs[0];
        let prevouts = [input.witness_utxo.clone().unwrap()];
        let leaf = TapLeafHash::from_script(&script, LeafVersion::TapScript);
        let script_signature = *input.tap_script_sigs.values().next().unwrap();
        let script_key = input.tap_script_sigs.keys().next().unwrap().0;
        let output_key =
            bitcoin::XOnlyPublicKey::from_slice(&prevouts[0].script_pubkey[2..]).unwrap();
        for (key, signature, path) in [
            (output_key, input.tap_key_sig.unwrap(), None),
            (script_key, script_signature, Some((leaf, u32::MAX))),
        ] {
            for (annex, valid) in [
                (Some(&[0x50, 1, 2, 3][..]), true),
                (None, false),
                (Some(&[0x50, 1, 2, 4][..]), false),
            ] {
                let hash = SighashCache::new(&psbt.unsigned_tx)
                    .taproot_signature_hash(
                        0,
                        &Prevouts::All(&prevouts),
                        annex.map(|value| Annex::new(value).unwrap()),
                        path,
                        hash_type,
                    )
                    .unwrap();
                assert_eq!(
                    secp.verify_schnorr(
                        &signature.sig,
                        &Message::from_digest_slice(&hash[..]).unwrap(),
                        &key
                    )
                    .is_ok(),
                    valid
                );
            }
        }
    }
}

#[test]
fn finalization_preserves_the_exact_annex_for_key_and_script_paths() {
    for scriptpath in [false, true] {
        for hash_type in [SchnorrSighashType::Default, SchnorrSighashType::All] {
            let (psbt, script) = signed(scriptpath, hash_type);
            let mut finalized = finalize(psbt, &Secp256k1::new()).unwrap();
            let stack = finalized.inputs[0]
                .final_script_witness
                .as_ref()
                .unwrap()
                .to_vec();
            assert_eq!(stack.len(), if scriptpath { 4 } else { 2 });
            assert_eq!(stack.last().unwrap(), &[0x50, 1, 2, 3]);
            if scriptpath {
                assert_eq!(stack[1], script.as_bytes());
            }
            assert_eq!(
                annex::set(&mut finalized.inputs[0], None),
                Err(AnnexError::FinalizedInput)
            );
            finalized.inputs[0].proprietary.remove(&annex::field());
            assert_eq!(
                annex::get(&finalized.inputs[0]).unwrap(),
                Some(&[0x50, 1, 2, 3][..])
            );
            assert_eq!(
                finalize(finalized.clone(), &Secp256k1::new()).unwrap(),
                finalized
            );
        }
    }
}

#[test]
fn annex_mutation_removal_and_addition_invalidate_collected_signatures() {
    for scriptpath in [false, true] {
        let (psbt, _) = signed(scriptpath, SchnorrSighashType::All);
        for changed in [None, Some(vec![0x50, 7])] {
            let mut changed_psbt = psbt.clone();
            annex::set(&mut changed_psbt.inputs[0], changed).unwrap();
            let (retained, _) = finalize(changed_psbt.clone(), &Secp256k1::new()).unwrap_err();
            assert_eq!(retained, changed_psbt);
        }
        let (keys, mut no_annex, _) = fixture(None, 1);
        annex::set(&mut no_annex.inputs[0], None).unwrap();
        keys.sign_psbt_mut(&mut no_annex, &Secp256k1::new(), SchnorrSighashType::All)
            .unwrap();
        if scriptpath {
            no_annex.inputs[0].tap_key_sig = None;
        }
        annex::set(&mut no_annex.inputs[0], Some(vec![0x50])).unwrap();
        assert!(finalize(no_annex, &Secp256k1::new()).is_err());
    }
}

#[test]
fn completed_witnesses_cannot_bypass_signature_or_annex_validation() {
    for scriptpath in [false, true] {
        let (psbt, _) = signed(scriptpath, SchnorrSighashType::All);
        let finalized = finalize(psbt, &Secp256k1::new()).unwrap();
        let mut conflict = finalized.clone();
        conflict.inputs[0]
            .proprietary
            .insert(annex::field(), vec![0x50, 9]);
        assert_eq!(
            annex::get(&conflict.inputs[0]),
            Err(AnnexError::FinalizedMismatch)
        );
        assert!(finalize(conflict, &Secp256k1::new()).is_err());
        let mut corrupt = finalized.clone();
        let mut stack = corrupt.inputs[0]
            .final_script_witness
            .as_ref()
            .unwrap()
            .to_vec();
        stack[0][10] ^= 1;
        corrupt.inputs[0].final_script_witness = Some(Witness::from_vec(stack));
        assert!(finalize(corrupt, &Secp256k1::new()).is_err());
        let mut wrong_flag = finalized;
        wrong_flag.inputs[0].sighash_type = Some(SchnorrSighashType::Default.into());
        assert!(finalize(wrong_flag, &Secp256k1::new()).is_err());
    }
}

#[test]
fn invalid_control_block_does_not_consume_partial_signatures() {
    let (mut psbt, _) = signed(true, SchnorrSighashType::All);
    let (mut control, script) = psbt.inputs[0]
        .tap_scripts
        .iter()
        .next()
        .map(|(a, b)| (a.clone(), b.clone()))
        .unwrap();
    control.output_key_parity = match control.output_key_parity {
        bitcoin::secp256k1::Parity::Even => bitcoin::secp256k1::Parity::Odd,
        bitcoin::secp256k1::Parity::Odd => bitcoin::secp256k1::Parity::Even,
    };
    psbt.inputs[0].tap_scripts.clear();
    psbt.inputs[0].tap_scripts.insert(control, script);
    let (retained, _) = finalize(psbt.clone(), &Secp256k1::new()).unwrap_err();
    assert_eq!(retained, psbt);
}

#[test]
fn mixed_inputs_all_require_valid_executed_signatures() {
    let secp = Secp256k1::new();
    let (keys, mut psbt, _) = fixture(None, 2);
    annex::set(&mut psbt.inputs[1], None).unwrap();
    keys.sign_psbt_mut(&mut psbt, &secp, SchnorrSighashType::All)
        .unwrap();
    let finalized = finalize(psbt, &secp).unwrap();
    for index in 0..2 {
        let mut corrupt = finalized.clone();
        let mut stack = corrupt.inputs[index]
            .final_script_witness
            .as_ref()
            .unwrap()
            .to_vec();
        stack[0][20] ^= 1;
        corrupt.inputs[index].final_script_witness = Some(Witness::from_vec(stack));
        assert!(finalize(corrupt, &secp).is_err());
    }
}

#[test]
fn annex_rejects_non_taproot_and_conflicting_previous_outputs() {
    let (keys, mut psbt, _) = fixture(None, 1);
    psbt.inputs[0].witness_utxo.as_mut().unwrap().script_pubkey =
        Builder::new().push_int(1).into_script().to_v0_p2wsh();
    assert!(keys
        .sign_psbt_mut(&mut psbt, &Secp256k1::new(), SchnorrSighashType::All)
        .is_err());
    assert!(finalize(psbt, &Secp256k1::new()).is_err());
    let (mut psbt, _) = signed(false, SchnorrSighashType::All);
    psbt.inputs[0].non_witness_utxo = Some(psbt.unsigned_tx.clone());
    let (retained, _) = finalize(psbt.clone(), &Secp256k1::new()).unwrap_err();
    assert_eq!(retained, psbt);

    let (keys, mut psbt, _) = fixture(None, 1);
    let funding = Transaction {
        output: vec![psbt.inputs[0].witness_utxo.clone().unwrap()],
        ..psbt.unsigned_tx.clone()
    };
    psbt.unsigned_tx.input[0].previous_output = OutPoint {
        txid: funding.txid(),
        vout: 0,
    };
    psbt.inputs[0].non_witness_utxo = Some(funding);
    keys.sign_psbt_mut(&mut psbt, &Secp256k1::new(), SchnorrSighashType::All)
        .unwrap();
    assert!(finalize(psbt.clone(), &Secp256k1::new()).is_ok());
    let mut full_only = psbt.clone();
    full_only.inputs[0].witness_utxo = None;
    assert!(finalize(full_only, &Secp256k1::new()).is_ok());
    psbt.inputs[0].witness_utxo.as_mut().unwrap().value += 1;
    assert_eq!(
        finalize(psbt.clone(), &Secp256k1::new()).unwrap_err().0,
        psbt
    );
}

#[test]
fn future_leaf_versions_cannot_be_finalized_with_an_annex() {
    let secp = Secp256k1::new();
    let (keys, mut psbt, script) = fixture(None, 1);
    let key = keys.0[0].to_keypair(&secp).x_only_public_key().0;
    let version = LeafVersion::from_consensus(0xc2).unwrap();
    let spend = TaprootBuilder::new()
        .add_leaf_with_ver(0, script.clone(), version)
        .unwrap()
        .finalize(&secp, key)
        .unwrap();
    psbt.inputs[0].witness_utxo.as_mut().unwrap().script_pubkey =
        Script::new_v1_p2tr_tweaked(spend.output_key());
    let control = spend.control_block(&(script.clone(), version)).unwrap();
    psbt.inputs[0].tap_scripts.clear();
    psbt.inputs[0]
        .tap_scripts
        .insert(control.clone(), (script.clone(), version));
    assert_eq!(finalize(psbt.clone(), &secp).unwrap_err().0, psbt);
    psbt.inputs[0].final_script_witness = Some(Witness::from_vec(vec![
        script.into_bytes(),
        control.serialize(),
        vec![0x50, 1, 2, 3],
    ]));
    assert_eq!(finalize(psbt.clone(), &secp).unwrap_err().0, psbt);
}

#[test]
fn a_failed_annex_input_preserves_its_data_and_other_completed_inputs() {
    let secp = Secp256k1::new();
    let (keys, mut psbt, _) = fixture(None, 2);
    keys.sign_psbt_mut(&mut psbt, &secp, SchnorrSighashType::All)
        .unwrap();
    annex::set(&mut psbt.inputs[1], Some(vec![0x50, 9])).unwrap();
    let failed_input = psbt.inputs[1].clone();
    let (partial, errors) = finalize(psbt, &secp).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert!(partial.inputs[0].final_script_witness.is_some());
    assert_eq!(partial.inputs[1], failed_input);
}

#[test]
fn unsupported_code_separator_leaf_is_retained() {
    let secp = Secp256k1::new();
    let (keys, _, script) = fixture(None, 1);
    let script = Builder::new()
        .push_opcode(OP_CODESEPARATOR)
        .into_script()
        .into_bytes()
        .into_iter()
        .chain(script.into_bytes())
        .collect::<Vec<_>>();
    let (_, mut psbt, _) = fixture(Some(Script::from(script)), 1);
    keys.sign_psbt_mut(&mut psbt, &secp, SchnorrSighashType::All)
        .unwrap();
    psbt.inputs[0].tap_key_sig = None;
    assert_eq!(finalize(psbt.clone(), &secp).unwrap_err().0, psbt);
}

#[test]
fn annex_ctv_uses_all_final_scriptsigs_and_rechecks_completed_inputs() {
    let secp = Secp256k1::new();
    let (keys, mut psbt, _) = fixture(None, 2);
    let redeem = Builder::new().push_int(1).into_script();
    let final_script_sig = Builder::new().push_slice(redeem.as_bytes()).into_script();
    let mut legacy = bitcoin::psbt::Input::default();
    legacy.witness_utxo = Some(TxOut {
        value: 10_000,
        script_pubkey: redeem.to_p2sh(),
    });
    legacy.redeem_script = Some(redeem);
    psbt.inputs[1] = legacy;
    let mut final_tx = psbt.unsigned_tx.clone();
    final_tx.input[1].script_sig = final_script_sig.clone();
    let key = keys.0[0].to_keypair(&secp).x_only_public_key().0;
    let script = miniscript::Miniscript::<bitcoin::XOnlyPublicKey, miniscript::Tap>::from_str(
        &format!("and_v(txtmpl({}),pk({key}))", final_tx.get_ctv_hash(0)),
    )
    .unwrap()
    .encode();
    let (_, replacement, _) = fixture(Some(script), 2);
    psbt.inputs[0] = replacement.inputs[0].clone();
    keys.sign_psbt_input_mut(&mut psbt, &secp, 0, SchnorrSighashType::All)
        .unwrap();
    psbt.inputs[0].tap_key_sig = None;
    let finalized = finalize(psbt, &secp).unwrap();
    assert_eq!(finalized.inputs[1].final_script_sig, Some(final_script_sig));
    assert_eq!(
        finalized.inputs[0]
            .final_script_witness
            .as_ref()
            .unwrap()
            .len(),
        4
    );
    let mut changed = finalized.clone();
    changed.inputs[1].final_script_sig = Some(Builder::new().push_int(0).push_int(1).into_script());
    assert!(finalize(changed, &secp).is_err());

    // A fresh, correct annex signature cannot authorize the wrong template.
    final_tx.lock_time += 1;
    let script = miniscript::Miniscript::<bitcoin::XOnlyPublicKey, miniscript::Tap>::from_str(
        &format!("and_v(txtmpl({}),pk({key}))", final_tx.get_ctv_hash(0)),
    )
    .unwrap()
    .encode();
    let (_, replacement, _) = fixture(Some(script), 2);
    let mut wrong_template = finalized;
    wrong_template.inputs[0] = replacement.inputs[0].clone();
    keys.sign_psbt_input_mut(&mut wrong_template, &secp, 0, SchnorrSighashType::All)
        .unwrap();
    wrong_template.inputs[0].tap_key_sig = None;
    assert_eq!(
        finalize(wrong_template.clone(), &secp).unwrap_err().0,
        wrong_template
    );
}
