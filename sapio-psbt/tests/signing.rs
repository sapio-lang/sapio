use bitcoin::bip32::{DerivationPath, Xpriv};
use bitcoin::blockdata::opcodes::all::OP_CHECKSIG;
use bitcoin::blockdata::script::Builder;
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::{Message, Secp256k1};
use bitcoin::sighash::{Prevouts, SighashCache, SingleMissingOutputError, TaprootError};
use bitcoin::taproot::{LeafVersion, TapLeafHash, TaprootBuilder};
use bitcoin::{Network, OutPoint, ScriptBuf, TapSighashType, Transaction, TxIn, TxOut};
use sapio_psbt::external_api::{finalize_psbt_format_api, PSBTApi};
use sapio_psbt::{PSBTSigningError, PSBTValidationError, SigningKey};

fn two_input_psbt() -> (SigningKey, Psbt, Vec<TxOut>) {
    let secp = Secp256k1::new();
    let root = Xpriv::new_master(Network::Regtest, &[42; 32]).unwrap();
    let key = root.to_keypair(&secp).x_only_public_key().0;
    let script = Builder::new()
        .push_slice(key.serialize())
        .push_opcode(OP_CHECKSIG)
        .into_script();
    let leaf = TapLeafHash::from_script(&script, LeafVersion::TapScript);
    let spend = TaprootBuilder::new()
        .add_leaf(0, script.clone())
        .unwrap()
        .finalize(&secp, key)
        .unwrap();
    let prevouts = vec![
        TxOut {
            value: bitcoin::Amount::from_sat(10_000),
            script_pubkey: ScriptBuf::new_p2tr_tweaked(spend.output_key()),
        };
        2
    ];
    let tx = Transaction {
        version: bitcoin::transaction::Version(2),
        lock_time: bitcoin::absolute::LockTime::from_consensus(0),
        input: (0..2)
            .map(|vout| TxIn {
                previous_output: OutPoint {
                    vout,
                    ..OutPoint::default()
                },
                ..TxIn::default()
            })
            .collect(),
        output: vec![TxOut {
            value: bitcoin::Amount::from_sat(19_000),
            script_pubkey: prevouts[0].script_pubkey.clone(),
        }],
    };
    let mut psbt = Psbt::from_unsigned_tx(tx).unwrap();
    for (input, prevout) in psbt.inputs.iter_mut().zip(&prevouts) {
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
    }
    (SigningKey(vec![root]), psbt, prevouts)
}

#[test]
fn signs_each_input_for_both_taproot_spending_paths() {
    let secp = Secp256k1::new();
    let (keys, mut psbt, prevouts) = two_input_psbt();
    let hash_type = TapSighashType::All;
    keys.sign_psbt_mut(&mut psbt, &secp, hash_type).unwrap();
    let mut cache = SighashCache::new(&psbt.unsigned_tx);
    for (index, input) in psbt.inputs.iter().enumerate() {
        let digest = cache
            .taproot_key_spend_signature_hash(index, &Prevouts::All(&prevouts), hash_type)
            .unwrap();
        let output_key =
            bitcoin::XOnlyPublicKey::from_slice(&prevouts[index].script_pubkey.as_bytes()[2..])
                .unwrap();
        secp.verify_schnorr(
            &input.tap_key_sig.unwrap().signature,
            &Message::from_digest_slice(&digest[..]).unwrap(),
            &output_key,
        )
        .unwrap();

        assert_eq!(input.tap_script_sigs.len(), 1);
        for ((key, leaf), signature) in &input.tap_script_sigs {
            let digest = cache
                .taproot_script_spend_signature_hash(
                    index,
                    &Prevouts::All(&prevouts),
                    *leaf,
                    hash_type,
                )
                .unwrap();
            secp.verify_schnorr(
                &signature.signature,
                &Message::from_digest_slice(&digest[..]).unwrap(),
                key,
            )
            .unwrap();
        }
    }
}

#[test]
fn rejects_single_without_a_corresponding_output() {
    let (keys, mut psbt, _) = two_input_psbt();
    let error = keys
        .sign_psbt_input_mut(&mut psbt, &Secp256k1::new(), 1, TapSighashType::Single)
        .unwrap_err();
    assert!(matches!(
        error,
        PSBTSigningError::Sighash(TaprootError::SingleMissingOutput(
            SingleMissingOutputError {
                input_index: 1,
                outputs_length: 1,
                ..
            }
        ))
    ));
    assert!(psbt.inputs[1].tap_key_sig.is_none());
    assert!(psbt.inputs[1].tap_script_sigs.is_empty());
}

fn assert_rejected_before_signing_or_finalizing(
    keys: &SigningKey,
    psbt: Psbt,
    expected: PSBTValidationError,
) {
    let secp = Secp256k1::new();
    for selected_input in [None, Some(0)] {
        let mut candidate = psbt.clone();
        let error = match selected_input {
            None => keys.sign_psbt_mut(&mut candidate, &secp, TapSighashType::All),
            Some(index) => {
                keys.sign_psbt_input_mut(&mut candidate, &secp, index, TapSighashType::All)
            }
        }
        .unwrap_err();
        assert!(
            matches!(error, PSBTSigningError::InvalidPSBT(ref actual) if *actual == expected),
            "unexpected signing error: {error}"
        );
        assert_eq!(candidate, psbt, "rejection must not add any signatures");
    }
    let Err(error) = finalize_psbt_format_api(psbt) else {
        panic!("finalizer accepted a malformed PSBT");
    };
    assert_eq!(error, expected);
}

#[test]
fn rejects_empty_transactions_before_signing_or_finalizing() {
    let (keys, mut psbt, _) = two_input_psbt();
    psbt.unsigned_tx.input.clear();
    psbt.inputs.clear();
    assert_rejected_before_signing_or_finalizing(&keys, psbt, PSBTValidationError::NoInputs);
}

#[test]
fn rejects_missing_and_extra_input_maps_before_signing() {
    for map_count in [0, 1, 3] {
        let (keys, mut psbt, _) = two_input_psbt();
        psbt.inputs.resize_with(map_count, Default::default);
        assert_rejected_before_signing_or_finalizing(
            &keys,
            psbt,
            PSBTValidationError::InputMapCount {
                transaction: 2,
                maps: map_count,
            },
        );
    }
}

#[test]
fn rejects_missing_and_extra_output_maps_before_signing() {
    for map_count in [0, 2] {
        let (keys, mut psbt, _) = two_input_psbt();
        psbt.outputs.resize_with(map_count, Default::default);
        assert_rejected_before_signing_or_finalizing(
            &keys,
            psbt,
            PSBTValidationError::OutputMapCount {
                transaction: 1,
                maps: map_count,
            },
        );
    }
}

#[test]
fn rejects_scriptsig_in_unsigned_transaction_before_signing() {
    let (keys, mut psbt, _) = two_input_psbt();
    psbt.unsigned_tx.input[1].script_sig = Builder::new().push_int(1).into_script();
    assert_rejected_before_signing_or_finalizing(
        &keys,
        psbt,
        PSBTValidationError::UnsignedTxHasScriptSig(1),
    );
}

#[test]
fn rejects_witness_in_unsigned_transaction_before_signing() {
    let (keys, mut psbt, _) = two_input_psbt();
    psbt.unsigned_tx.input[1].witness.push([1]);
    assert_rejected_before_signing_or_finalizing(
        &keys,
        psbt,
        PSBTValidationError::UnsignedTxHasWitness(1),
    );
}

#[test]
fn finalizer_distinguishes_missing_signatures_from_complete_psbts() {
    let (keys, mut psbt, _) = two_input_psbt();
    assert!(matches!(
        finalize_psbt_format_api(psbt.clone()).unwrap(),
        PSBTApi::NotFinished {
            completed: false,
            ..
        }
    ));
    keys.sign_psbt_mut(&mut psbt, &Secp256k1::new(), TapSighashType::All)
        .unwrap();
    assert!(matches!(
        finalize_psbt_format_api(psbt).unwrap(),
        PSBTApi::Finished {
            completed: true,
            ..
        }
    ));
}
