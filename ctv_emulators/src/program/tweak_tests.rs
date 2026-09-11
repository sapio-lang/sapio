use super::*;
use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::Parity;
use bitcoin::taproot::TapTweakHash;
use bitcoin::{Network, Transaction, TxIn};
use sapio_base::fragments::{
    template_hash, template_signed_by, KnownTweakError, KnownTweakProof, TemplateKey,
};

#[test]
fn emulator_root_authorizes_its_bip32_program_child_for_every_parity_combination() {
    // The instance and derivation path exist before the witness. Neither the
    // root signature nor the opening scalar participates in the program ID.
    let tree =
        TapNodeHash::from_byte_array(sha256::Hash::hash(b"known-tweak tree").to_byte_array());
    let secp = Secp256k1::new();
    let mut seen = [[false; 2]; 2];
    for seed in 0..64u8 {
        let root = Xpriv::new_master(Network::Testnet, &[seed; 32]).unwrap();
        let public_root = Xpub::from_priv(&secp, &root);
        let program = template_signed_by(TemplateKey::KnownTweak, public_root).unwrap();
        let instance = program.instance();
        let (root_key, root_parity) = public_root.public_key.x_only_public_key();
        let child = public_root
            .derive_pub(&secp, &program_derivation_path(instance.id()))
            .unwrap();
        let (internal, child_parity) = child.public_key.x_only_public_key();
        assert_eq!(internal, program.derive_public_key().unwrap());
        let slot = (root_parity.to_u8() as usize, child_parity.to_u8() as usize);
        if seen[slot.0][slot.1] {
            continue;
        }
        assert_ne!(root_key, internal);
        let oracle = ProgramOracle::new(root, vec![]).unwrap();

        for merkle_root in [None, Some(tree)] {
            let tap = TapTweakHash::from_key_and_tweak(internal, merkle_root).to_scalar();
            let (output_key, _) = internal.add_tweak(&secp, &tap).unwrap();
            let proof = KnownTweakProof::for_program(&program, merkle_root).unwrap();
            assert_eq!(proof.key(), root_key);
            proof.check_output(output_key).unwrap();
            // Independently reconstruct the full point, including the proof's
            // claimed parity, against the separately derived Taproot output.
            let opened = root_key
                .public_key(Parity::Even)
                .add_exp_tweak(&secp, &proof.tweak())
                .unwrap();
            assert_eq!(opened, output_key.public_key(proof.parity()));
            assert_eq!(
                proof.check_output(root_key),
                Err(KnownTweakError::OutputKeyMismatch)
            );
            let other_tree = if merkle_root.is_some() {
                None
            } else {
                Some(tree)
            };
            assert_eq!(
                KnownTweakProof::for_program(&program, other_tree)
                    .unwrap()
                    .check_output(output_key),
                Err(KnownTweakError::OutputKeyMismatch)
            );

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
                witness: proof.witness(&signature),
                path: ProgramSpendPath::KeyPath,
                psbt: PSBT(psbt),
            };
            let signed = oracle.sign(request.clone()).unwrap();
            validate_program_response(&request, &signed, &public_root).unwrap();
            assert!(signed.inputs[0].tap_key_sig.is_some());
            assert!(signed.inputs[0].tap_internal_key.is_none());
            assert!(signed.inputs[0].tap_scripts.is_empty());

            let mut wrong_tweak = request.clone();
            wrong_tweak.witness[32] ^= 1;
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
