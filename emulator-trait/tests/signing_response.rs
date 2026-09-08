use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{Keypair, Message, Secp256k1, SecretKey};
use bitcoin::util::bip32::{DerivationPath, ExtendedPrivKey, ExtendedPubKey, Fingerprint};
use bitcoin::util::psbt::{raw, PartiallySignedTransaction as Psbt};
use bitcoin::util::taproot::{LeafVersion, TapLeafHash};
use bitcoin::{EcdsaSig, EcdsaSighashType, PublicKey, SchnorrSig, SchnorrSighashType};
use bitcoin::{Network, Script, Transaction, TxIn, TxOut, Witness};
use sapio_ctv_emulator_trait::{
    sign_checked, validate_signing_response, CTVAvailable, CTVEmulator, Clause, EmulatorError,
};

type Mutation = fn(&mut Psbt);

fn add_signatures(psbt: &mut Psbt, input: usize, byte: u8) {
    let secp = Secp256k1::new();
    let secret = SecretKey::from_slice(&[byte; 32]).unwrap();
    let keypair = Keypair::from_secret_key(&secp, &secret);
    let message = Message::from_digest_slice(&[byte; 32]).unwrap();
    let schnorr = SchnorrSig {
        sig: secp.sign_schnorr_no_aux_rand(&message, &keypair),
        hash_ty: SchnorrSighashType::Default,
    };
    psbt.inputs[input].partial_sigs.insert(
        PublicKey::new(keypair.public_key()),
        EcdsaSig {
            sig: secp.sign_ecdsa(&message, &secret),
            hash_ty: EcdsaSighashType::All,
        },
    );
    psbt.inputs[input].tap_key_sig.get_or_insert(schnorr);
    psbt.inputs[input].tap_script_sigs.insert(
        (
            keypair.x_only_public_key().0,
            TapLeafHash::from_script(&Script::from(vec![byte]), LeafVersion::TapScript),
        ),
        schnorr,
    );
}

fn request() -> Psbt {
    let mut psbt = Psbt::from_unsigned_tx(Transaction {
        version: 2,
        lock_time: 0,
        input: vec![TxIn::default(), TxIn::default()],
        output: vec![TxOut {
            value: 9_000,
            script_pubkey: Script::from(vec![0x51]),
        }],
    })
    .unwrap();
    psbt.inputs[0].witness_utxo = Some(TxOut {
        value: 10_000,
        script_pubkey: Script::from(vec![0x51]),
    });
    psbt.inputs[0].non_witness_utxo = Some(psbt.unsigned_tx.clone());
    psbt.inputs[0].sighash_type = Some(SchnorrSighashType::Default.into());
    psbt.inputs[0].witness_script = Some(Script::from(vec![0x51]));
    psbt.inputs[0].redeem_script = Some(Script::from(vec![0x51]));
    psbt.inputs[0].final_script_sig = Some(Script::new());
    psbt.inputs[0].final_script_witness = Some(Witness::from_vec(vec![vec![1]]));
    let unknown = raw::Key {
        type_value: 0x80,
        key: vec![1],
    };
    let proprietary = raw::ProprietaryKey {
        prefix: b"test".to_vec(),
        subtype: 1,
        key: vec![2],
    };
    psbt.unknown.insert(unknown.clone(), vec![3]);
    psbt.proprietary.insert(proprietary.clone(), vec![4]);
    psbt.inputs[0].unknown.insert(unknown.clone(), vec![5]);
    psbt.inputs[0]
        .proprietary
        .insert(proprietary.clone(), vec![6]);
    psbt.outputs[0].unknown.insert(unknown, vec![7]);
    psbt.outputs[0].proprietary.insert(proprietary, vec![8]);
    add_signatures(&mut psbt, 0, 1);
    psbt
}

struct Signer(fn(&mut Psbt));

impl CTVEmulator for Signer {
    fn get_signer_for(&self, _: sha256::Hash) -> Result<Clause, EmulatorError> {
        Ok(Clause::Trivial)
    }

    fn sign(&self, mut psbt: Psbt) -> Result<Psbt, EmulatorError> {
        (self.0)(&mut psbt);
        Ok(psbt)
    }
}

#[test]
fn checked_signing_preserves_complete_requests_and_accepts_only_signature_additions() {
    let original = request();
    assert_eq!(
        sign_checked(&CTVAvailable, original.clone()).unwrap(),
        original
    );
    let signer = Signer(|psbt| {
        add_signatures(psbt, 0, 2);
        add_signatures(psbt, 1, 3);
    });
    let response = sign_checked(&signer, original.clone()).unwrap();
    assert_eq!(response.inputs[0].partial_sigs.len(), 2);
    assert_eq!(response.inputs[0].tap_script_sigs.len(), 2);
    assert_eq!(
        response.inputs[0].tap_key_sig,
        original.inputs[0].tap_key_sig
    );
    assert!(response.inputs[1].tap_key_sig.is_some());
    validate_signing_response(&original, &response).unwrap();
}

#[test]
fn checked_signing_rejects_changes_to_each_protected_psbt_field_family() {
    let changes: &[(&str, Mutation)] = &[
        ("transaction", |p| p.unsigned_tx.output[0].value -= 1),
        ("input count", |p| {
            p.inputs.pop();
        }),
        ("output count", |p| p.outputs.clear()),
        ("unsigned input count", |p| {
            p.unsigned_tx.input.pop();
        }),
        ("unsigned output count", |p| p.unsigned_tx.output.clear()),
        ("PSBT version", |p| p.version += 1),
        ("global xpub", |p| {
            let secp = Secp256k1::new();
            let root = ExtendedPrivKey::new_master(Network::Regtest, &[3; 32]).unwrap();
            p.xpub.insert(
                ExtendedPubKey::from_priv(&secp, &root),
                (Fingerprint::default(), DerivationPath::default()),
            );
        }),
        ("witness UTXO", |p| {
            p.inputs[0].witness_utxo.as_mut().unwrap().value -= 1
        }),
        ("nonwitness UTXO", |p| p.inputs[0].non_witness_utxo = None),
        ("sighash", |p| {
            p.inputs[0].sighash_type = Some(SchnorrSighashType::None.into())
        }),
        ("redeem script", |p| p.inputs[0].redeem_script = None),
        ("witness script", |p| p.inputs[0].witness_script = None),
        ("final scriptSig", |p| p.inputs[0].final_script_sig = None),
        ("final witness", |p| p.inputs[0].final_script_witness = None),
        ("input derivation", |p| {
            let key = p.inputs[0].partial_sigs.keys().next().unwrap().inner;
            p.inputs[0]
                .bip32_derivation
                .insert(key, (Fingerprint::default(), DerivationPath::default()));
        }),
        ("taproot internal key", |p| {
            p.inputs[0].tap_internal_key =
                Some(p.inputs[0].tap_script_sigs.keys().next().unwrap().0)
        }),
        ("preimage", |p| {
            p.inputs[0]
                .sha256_preimages
                .insert(sha256::Hash::hash(&[1]), vec![1]);
        }),
        ("global unknown", |p| p.unknown.clear()),
        ("global proprietary", |p| p.proprietary.clear()),
        ("input unknown", |p| p.inputs[0].unknown.clear()),
        ("input proprietary", |p| p.inputs[0].proprietary.clear()),
        ("output unknown", |p| p.outputs[0].unknown.clear()),
        ("output proprietary", |p| p.outputs[0].proprietary.clear()),
        ("output script", |p| {
            p.outputs[0].witness_script = Some(Script::new())
        }),
        ("second input", |p| {
            p.inputs[1].sighash_type = Some(SchnorrSighashType::All.into())
        }),
    ];
    for (name, change) in changes {
        assert!(
            matches!(
                sign_checked(&Signer(*change), request()),
                Err(EmulatorError::InvalidResponse)
            ),
            "{}",
            name
        );
    }
}

#[test]
fn checked_signing_rejects_removing_or_replacing_existing_signatures() {
    let changes: &[fn(&mut Psbt)] = &[
        |p| p.inputs[0].partial_sigs.clear(),
        |p| {
            p.inputs[0]
                .partial_sigs
                .values_mut()
                .next()
                .unwrap()
                .hash_ty = EcdsaSighashType::None
        },
        |p| p.inputs[0].tap_key_sig = None,
        |p| p.inputs[0].tap_key_sig.as_mut().unwrap().hash_ty = SchnorrSighashType::All,
        |p| p.inputs[0].tap_script_sigs.clear(),
        |p| {
            p.inputs[0]
                .tap_script_sigs
                .values_mut()
                .next()
                .unwrap()
                .hash_ty = SchnorrSighashType::All
        },
    ];
    for change in changes {
        assert!(matches!(
            sign_checked(&Signer(*change), request()),
            Err(EmulatorError::InvalidResponse)
        ));
    }
}
