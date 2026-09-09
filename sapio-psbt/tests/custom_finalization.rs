use bitcoin::blockdata::opcodes::all::{OP_ADD, OP_CHECKSIG, OP_NUMEQUALVERIFY};
use bitcoin::blockdata::script::Builder;
use bitcoin::consensus::deserialize;
use bitcoin::hashes::hex::FromHex;
use bitcoin::secp256k1::{Keypair, Message, Secp256k1, SecretKey};
use bitcoin::util::psbt::PartiallySignedTransaction;
use bitcoin::util::sighash::{Prevouts, SighashCache};
use bitcoin::util::taproot::{LeafVersion, TapLeafHash, TaprootBuilder};
use bitcoin::{OutPoint, SchnorrSig, SchnorrSighashType, Script, Transaction, TxIn, TxOut};
use miniscript::{Miniscript, Tap};
use sapio_psbt::external_api::{finalize_psbt_format_api, PSBTApi};
use sapio_psbt::PSBTValidationError;

fn keypair(byte: u8) -> Keypair {
    SecretKey::from_slice(&[byte; 32])
        .unwrap()
        .keypair(&Secp256k1::new())
}

fn fixture(include_native_leaf: bool) -> (PartiallySignedTransaction, Script, Script) {
    let secp = Secp256k1::new();
    let signer = keypair(1).x_only_public_key().0;
    // Witness [signature, 2, 3] satisfies this script, but the arithmetic
    // fragment is outside the native Miniscript satisfier's language.
    let raw = Builder::new()
        .push_opcode(OP_ADD)
        .push_int(5)
        .push_opcode(OP_NUMEQUALVERIFY)
        .push_slice(&signer.serialize())
        .push_opcode(OP_CHECKSIG)
        .into_script();
    let native = Builder::new()
        .push_slice(&signer.serialize())
        .push_opcode(OP_CHECKSIG)
        .into_script();
    assert!(Miniscript::<bitcoin::XOnlyPublicKey, Tap>::parse_insane(&raw).is_err());
    assert!(Miniscript::<bitcoin::XOnlyPublicKey, Tap>::parse_insane(&native).is_ok());
    let scripts = if include_native_leaf {
        vec![raw.clone(), native.clone()]
    } else {
        vec![raw.clone()]
    };
    let depth = u8::from(include_native_leaf);
    let mut builder = TaprootBuilder::new();
    for script in &scripts {
        builder = builder.add_leaf(depth, script.clone()).unwrap();
    }
    // No key-path signature is supplied: the selected script must authorize
    // finalization, independently of whether the tree has another leaf.
    let spend = builder
        .finalize(&secp, keypair(2).x_only_public_key().0)
        .unwrap();
    let prevout = TxOut {
        value: 1_000,
        script_pubkey: Script::new_v1_p2tr_tweaked(spend.output_key()),
    };
    let tx = Transaction {
        version: 2,
        lock_time: 0,
        input: vec![TxIn {
            previous_output: OutPoint::default(),
            ..Default::default()
        }],
        output: vec![TxOut {
            value: 900,
            script_pubkey: native.clone().to_v0_p2wsh(),
        }],
    };
    let mut psbt = PartiallySignedTransaction::from_unsigned_tx(tx).unwrap();
    let input = &mut psbt.inputs[0];
    input.witness_utxo = Some(prevout);
    input.tap_internal_key = Some(spend.internal_key());
    input.tap_merkle_root = spend.merkle_root();
    for script in scripts {
        let leaf = (script, LeafVersion::TapScript);
        input
            .tap_scripts
            .insert(spend.control_block(&leaf).unwrap(), leaf);
    }
    (psbt, raw, native)
}

fn sign_leaf(psbt: &mut PartiallySignedTransaction, script: &Script) -> SchnorrSig {
    let secp = Secp256k1::new();
    let key = keypair(1);
    let leaf = TapLeafHash::from_script(script, LeafVersion::TapScript);
    let prevouts = [psbt.inputs[0].witness_utxo.clone().unwrap()];
    let digest = SighashCache::new(&psbt.unsigned_tx)
        .taproot_script_spend_signature_hash(
            0,
            &Prevouts::All(&prevouts),
            leaf,
            SchnorrSighashType::Default,
        )
        .unwrap();
    let message = Message::from_digest_slice(&digest[..]).unwrap();
    let signature = SchnorrSig {
        sig: secp.sign_schnorr_no_aux_rand(&message, &key),
        hash_ty: SchnorrSighashType::Default,
    };
    secp.verify_schnorr(&signature.sig, &message, &key.x_only_public_key().0)
        .unwrap();
    psbt.inputs[0]
        .tap_script_sigs
        .insert((key.x_only_public_key().0, leaf), signature);
    signature
}

#[test]
fn unsupported_custom_leaf_keeps_its_script_proof_and_valid_partial_signature() {
    let (mut original, raw, _) = fixture(false);
    sign_leaf(&mut original, &raw);
    let PSBTApi::NotFinished {
        completed,
        psbt,
        errors,
        ..
    } = finalize_psbt_format_api(original.clone()).unwrap()
    else {
        panic!("custom arithmetic leaf must require an external satisfier");
    };
    assert!(!completed);
    let restored: PartiallySignedTransaction = deserialize(&base64::decode(psbt).unwrap()).unwrap();
    assert_eq!(restored, original);
    let leaf = TapLeafHash::from_script(&raw, LeafVersion::TapScript);
    assert!(errors.contains(&format!(
        "Input 0: unsupported custom tapscript leaf {leaf}; spending this leaf requires an external satisfier"
    )));
}

#[test]
fn a_signed_native_leaf_finalizes_even_when_a_custom_leaf_is_present() {
    let (mut psbt, raw, native) = fixture(true);
    sign_leaf(&mut psbt, &raw);
    let signature = sign_leaf(&mut psbt, &native);
    let PSBTApi::Finished { completed, hex } = finalize_psbt_format_api(psbt).unwrap() else {
        panic!("the recognized signed alternative must remain usable");
    };
    assert!(completed);
    let tx: Transaction = deserialize(&Vec::<u8>::from_hex(&hex).unwrap()).unwrap();
    let witness = tx.input[0].witness.to_vec();
    assert_eq!(witness.len(), 3);
    assert_eq!(witness[0], signature.to_vec());
    assert_eq!(witness[1], native.as_bytes());
}

#[test]
fn custom_leaves_do_not_hide_structurally_malformed_psbt_maps() {
    let (mut psbt, _, _) = fixture(false);
    psbt.inputs.clear();
    match finalize_psbt_format_api(psbt) {
        Err(error) => assert_eq!(
            error,
            PSBTValidationError::InputMapCount {
                transaction: 1,
                maps: 0,
            }
        ),
        Ok(_) => panic!("malformed maps must fail structural validation"),
    }
}
