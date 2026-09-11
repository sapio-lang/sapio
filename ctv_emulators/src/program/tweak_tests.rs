use super::*;
use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{Parity, Scalar, SecretKey};
use bitcoin::taproot::TapTweakHash;
use bitcoin::{Network, Transaction, TxIn};
use sapio_base::fragments::{
    known_tweak_witness, template_authorization_wasm_instance, template_hash, TemplateKey,
};

// These are public scalars. Reuse the pinned curve arithmetic while handling
// zero explicitly, since SecretKey intentionally cannot represent zero.
fn negate(value: Scalar) -> Scalar {
    if value == Scalar::ZERO {
        value
    } else {
        Scalar::from(
            SecretKey::from_slice(&value.to_be_bytes())
                .unwrap()
                .negate(),
        )
    }
}

fn add(left: Scalar, right: Scalar) -> Scalar {
    if left == Scalar::ZERO {
        right
    } else if negate(left) == right {
        Scalar::ZERO
    } else {
        Scalar::from(
            SecretKey::from_slice(&left.to_be_bytes())
                .unwrap()
                .add_tweak(&right)
                .expect("canonical operands with a nonzero sum"),
        )
    }
}

fn with_sign(value: Scalar, sign: Parity) -> Scalar {
    if sign == Parity::Odd {
        negate(value)
    } else {
        value
    }
}

fn public_child_and_tweak(root: Xpub, instance: &ProgramInstance) -> (Xpub, Scalar) {
    let secp = Secp256k1::verification_only();
    let path = program_derivation_path(instance.id());
    let mut current = root;
    let mut cumulative = Scalar::ZERO;
    for child in &path {
        let (tweak, _) = current.ckd_pub_tweak(*child).unwrap();
        cumulative = add(cumulative, Scalar::from(tweak));
        current = current.ckd_pub(&secp, *child).unwrap();
    }
    assert_eq!(current, root.derive_pub(&secp, &path).unwrap());
    assert_eq!(
        current.to_x_only_pub(),
        instance.derive_public_key(&root).unwrap()
    );
    (current, cumulative)
}

#[test]
fn emulator_root_authorizes_its_bip32_program_child_for_every_parity_combination() {
    assert_eq!(add(Scalar::ZERO, Scalar::ZERO), Scalar::ZERO);
    assert_eq!(add(Scalar::ONE, Scalar::MAX), Scalar::ZERO);
    assert_eq!(negate(Scalar::ZERO), Scalar::ZERO);

    // The instance and derivation path exist before the witness. Neither the
    // root signature nor the opening scalar participates in the program ID.
    let instance = template_authorization_wasm_instance(TemplateKey::KnownTweak);
    let tree =
        TapNodeHash::from_byte_array(sha256::Hash::hash(b"known-tweak tree").to_byte_array());
    let secp = Secp256k1::new();
    let mut seen = [[false; 2]; 2];
    for seed in 0..64u8 {
        let root = Xpriv::new_master(Network::Testnet, &[seed; 32]).unwrap();
        let public_root = Xpub::from_priv(&secp, &root);
        let (root_key, root_parity) = public_root.public_key.x_only_public_key();
        let (child, cumulative) = public_child_and_tweak(public_root, &instance);
        let (internal, child_parity) = child.public_key.x_only_public_key();
        let slot = (root_parity.to_u8() as usize, child_parity.to_u8() as usize);
        if seen[slot.0][slot.1] {
            continue;
        }
        assert_ne!(root_key, internal);
        let oracle = ProgramOracle::new(root, vec![]).unwrap();

        for merkle_root in [None, Some(tree)] {
            let tap = TapTweakHash::from_key_and_tweak(internal, merkle_root).to_scalar();
            let (output_key, output_parity) = internal.add_tweak(&secp, &tap).unwrap();
            // Let r/c be +1 for an even root/child and -1 for an odd one.
            // The full BIP32 child is root + C*G. After x-only normalization
            // and TapTweak, choose Q or -Q so P has coefficient +1:
            // t = r*C + (r*c)*Tap, with Q parity flipped when r*c = -1.
            let output_sign = root_parity ^ child_parity;
            let tweak = add(
                with_sign(cumulative, root_parity),
                with_sign(tap, output_sign),
            );
            let proof_parity = output_parity ^ output_sign;
            assert!(root_key.tweak_add_check(&secp, &output_key, proof_parity, tweak));

            let mut psbt = Psbt::from_unsigned_tx(Transaction {
                version: bitcoin::transaction::Version(2),
                lock_time: bitcoin::absolute::LockTime::from_consensus(40),
                input: vec![TxIn {
                    previous_output: OutPoint {
                        txid: bitcoin::Txid::from_byte_array([3; 32]),
                        vout: 0,
                    },
                    sequence: bitcoin::Sequence(42),
                    ..TxIn::default()
                }],
                output: vec![TxOut {
                    value: bitcoin::Amount::from_sat(9_000),
                    script_pubkey: ScriptBuf::from(vec![0x51]),
                }],
            })
            .unwrap();
            psbt.inputs[0].witness_utxo = Some(TxOut {
                value: bitcoin::Amount::from_sat(10_000),
                script_pubkey: ScriptBuf::new_p2tr(&secp, internal, merkle_root),
            });
            psbt.inputs[0].tap_merkle_root = merkle_root;
            assert!(psbt.inputs[0].tap_internal_key.is_none());
            assert!(psbt.inputs[0].tap_scripts.is_empty());
            assert!(psbt.inputs[0].tap_key_origins.is_empty());

            let hash = template_hash(&psbt.unsigned_tx, 0, None).unwrap();
            let message = Message::from_digest_slice(hash.as_ref()).unwrap();
            let signature = secp.sign_schnorr_no_aux_rand(&message, &root.to_keypair(&secp));
            secp.verify_schnorr(&signature, &message, &root_key)
                .unwrap();
            assert!(secp
                .verify_schnorr(&signature, &message, &internal)
                .is_err());
            let request = ProgramSigningRequest {
                instance: instance.clone(),
                input_index: 0,
                witness: known_tweak_witness(root_key, tweak, proof_parity, Some(&signature)),
                path: ProgramSpendPath::KeyPath,
                psbt: PSBT(psbt),
            };
            let signed = oracle.sign(request.clone()).unwrap();
            validate_program_response(&request, &signed, &public_root).unwrap();
            assert!(signed.inputs[0].tap_key_sig.is_some());
            assert!(signed.inputs[0].tap_internal_key.is_none());
            assert!(signed.inputs[0].tap_scripts.is_empty());

            let mut wrong_tweak = request.clone();
            wrong_tweak.witness = known_tweak_witness(
                root_key,
                add(tweak, Scalar::ONE),
                proof_parity,
                Some(&signature),
            );
            assert!(matches!(
                oracle.sign(wrong_tweak),
                Err(ProgramError::Evaluation(_))
            ));
            let mut wrong_parity = request.clone();
            wrong_parity.witness[64] ^= 1;
            assert!(matches!(
                oracle.sign(wrong_parity),
                Err(ProgramError::Evaluation(_))
            ));
            let mut wrong_signature = request;
            wrong_signature.witness[65] ^= 1;
            assert!(matches!(
                oracle.sign(wrong_signature),
                Err(ProgramError::Evaluation(_))
            ));
        }
        seen[slot.0][slot.1] = true;
        if seen == [[true; 2]; 2] {
            break;
        }
    }
    assert_eq!(
        seen, [[true; 2]; 2],
        "all root/child parity combinations must execute"
    );
}
