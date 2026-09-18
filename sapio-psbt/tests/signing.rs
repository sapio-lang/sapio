use bitcoin::bip32::{DerivationPath, Xpriv};
use bitcoin::blockdata::opcodes::all::OP_CHECKSIG;
use bitcoin::blockdata::script::Builder;
use bitcoin::key::TapTweak;
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

#[test]
fn rejects_taproot_metadata_that_does_not_match_the_previous_output() {
    let secp = Secp256k1::new();
    for wrong_script in [false, true] {
        let (keys, mut psbt, _) = two_input_psbt();
        keys.sign_psbt_mut(&mut psbt, &secp, TapSighashType::All)
            .unwrap();
        if wrong_script {
            psbt.inputs[0].witness_utxo.as_mut().unwrap().script_pubkey = ScriptBuf::new();
        } else {
            psbt.inputs[0].tap_merkle_root = None;
        }
        let original = psbt.clone();
        assert!(matches!(
            keys.sign_psbt_input_mut(&mut psbt, &secp, 0, TapSighashType::All),
            Err(PSBTSigningError::TaprootKeyMismatch(0))
        ));
        assert_eq!(
            psbt, original,
            "must preserve an existing signature on rejection"
        );
    }
}

#[test]
fn preserves_valid_existing_key_signatures_with_different_randomness() {
    let secp = Secp256k1::new();
    let (keys, mut psbt, prevouts) = two_input_psbt();
    let digest = SighashCache::new(&psbt.unsigned_tx)
        .taproot_key_spend_signature_hash(0, &Prevouts::All(&prevouts), TapSighashType::All)
        .unwrap();
    let key = keys.0[0]
        .to_keypair(&secp)
        .tap_tweak(&secp, psbt.inputs[0].tap_merkle_root)
        .to_keypair();
    let existing = bitcoin::taproot::Signature {
        signature: secp.sign_schnorr_with_aux_rand(
            &Message::from_digest_slice(&digest[..]).unwrap(),
            &key,
            &[7; 32],
        ),
        sighash_type: TapSighashType::All,
    };
    psbt.inputs[0].tap_key_sig = Some(existing);
    keys.sign_psbt_mut(&mut psbt, &secp, TapSighashType::All)
        .unwrap();
    assert_eq!(psbt.inputs[0].tap_key_sig, Some(existing));
}

#[test]
fn rejects_conflicting_key_signatures_without_overwriting_them() {
    let secp = Secp256k1::new();
    for different_sighash in [false, true] {
        let (keys, mut psbt, _) = two_input_psbt();
        let original_type = if different_sighash {
            TapSighashType::None
        } else {
            TapSighashType::All
        };
        keys.sign_psbt_mut(&mut psbt, &secp, original_type).unwrap();
        if !different_sighash {
            psbt.unsigned_tx.output[0].value = bitcoin::Amount::from_sat(18_000);
        }
        let original = psbt.clone();
        assert!(matches!(
            keys.sign_psbt_input_mut(&mut psbt, &secp, 0, TapSighashType::All),
            Err(PSBTSigningError::ConflictingTaprootKeySignature(0))
        ));
        assert_eq!(psbt, original);
    }
}

fn add_previous_transaction(psbt: &mut Psbt, prevouts: Vec<TxOut>) {
    let previous = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: vec![TxIn::default()],
        output: prevouts,
    };
    for (index, input) in psbt.inputs.iter_mut().enumerate() {
        input.non_witness_utxo = Some(previous.clone());
        psbt.unsigned_tx.input[index].previous_output =
            OutPoint::new(previous.compute_txid(), index as u32);
    }
}

#[test]
fn signs_using_authenticated_previous_transactions_without_witness_utxos() {
    let (keys, mut psbt, prevouts) = two_input_psbt();
    add_previous_transaction(&mut psbt, prevouts);
    for input in &mut psbt.inputs {
        input.witness_utxo = None;
    }
    keys.sign_psbt_mut(&mut psbt, &Secp256k1::new(), TapSighashType::All)
        .unwrap();
    assert!(psbt.inputs.iter().all(|input| input.tap_key_sig.is_some()));
}

#[test]
fn rejects_bad_funding_before_adding_any_signatures() {
    use sapio_base::psbt::FundingError;
    for case in 0..5 {
        let (keys, mut psbt, prevouts) = two_input_psbt();
        add_previous_transaction(&mut psbt, prevouts);
        let expected = match case {
            0 => {
                psbt.inputs[1].witness_utxo.as_mut().unwrap().value =
                    bitcoin::Amount::from_sat(9_999);
                PSBTValidationError::Funding(FundingError::ConflictingOutputs(1))
            }
            1 => {
                psbt.inputs[1].non_witness_utxo.as_mut().unwrap().output[0].value =
                    bitcoin::Amount::ZERO;
                PSBTValidationError::Funding(FundingError::PreviousTransaction(1))
            }
            2 => {
                psbt.unsigned_tx.input[1].previous_output.vout = 2;
                PSBTValidationError::Funding(FundingError::PreviousTransaction(1))
            }
            3 => {
                psbt.unsigned_tx.input[1].previous_output =
                    psbt.unsigned_tx.input[0].previous_output;
                PSBTValidationError::Funding(FundingError::DuplicateInput(1))
            }
            _ => {
                psbt.inputs[1].witness_utxo = None;
                psbt.inputs[1].non_witness_utxo = None;
                PSBTValidationError::MissingPreviousOutput(1)
            }
        };
        let original = psbt.clone();
        assert!(matches!(
            keys.sign_psbt_mut(&mut psbt, &Secp256k1::new(), TapSighashType::All),
            Err(PSBTSigningError::InvalidPSBT(actual)) if actual == expected
        ));
        assert_eq!(psbt, original);
    }
}
