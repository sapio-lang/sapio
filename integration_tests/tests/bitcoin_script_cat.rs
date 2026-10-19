use bitcoin::bip32::{Xpriv, Xpub};
use bitcoin::consensus::Encodable;
use bitcoin::hashes::{sha256, Hash};
use bitcoin::opcodes::all::{OP_CAT, OP_CHECKSIG, OP_EQUALVERIFY};
use bitcoin::psbt::Psbt;
use bitcoin::script::Builder;
use bitcoin::secp256k1::{Keypair, Message};
use bitcoin::sighash::{Annex, Prevouts, SighashCache};
use bitcoin::taproot::{LeafVersion, TapLeafHash, TaprootBuilder};
use bitcoin::{
    Amount, Network, OutPoint, ScriptBuf, Sequence, TapSighashType, Transaction, TxIn, TxOut,
};
use emulator_connect::program::{
    ProgramError, ProgramOracle, ProgramSigningRequest, ProgramSpendPath, PSBT,
};
use sapio_base::program::ProgramInstance;
use sapio_base::secp::secp;
use sapio_bitcoin_script_cat_evaluator::sighash::signature_hash;
use sapio_covenant_fragments::{Context, MAX_VIEW_BYTES};

const MODULE: &[u8] = include_bytes!("../../evaluators/artifacts/bitcoin_script_cat.wasm");

fn root() -> Xpriv {
    Xpriv::new_master(Network::Testnet, &[181; 32]).unwrap()
}

fn owner() -> Keypair {
    Xpriv::new_master(Network::Testnet, &[182; 32])
        .unwrap()
        .to_keypair(secp())
}

fn oracle() -> ProgramOracle {
    ProgramOracle::new(root(), vec![]).unwrap()
}

fn stack(elements: &[&[u8]]) -> Vec<u8> {
    let mut bytes = (elements.len() as u32).to_le_bytes().to_vec();
    for element in elements {
        bytes.extend_from_slice(&(element.len() as u32).to_le_bytes());
        bytes.extend_from_slice(element);
    }
    bytes
}

fn request(script: ScriptBuf, witness: Vec<u8>) -> ProgramSigningRequest {
    let instance = ProgramInstance::wasm_v2(MODULE.to_vec(), script.into_bytes()).unwrap();
    let key = instance
        .derive_public_key(&Xpub::from_priv(secp(), &root()))
        .unwrap();
    let wrapper = Builder::new()
        .push_x_only_key(&key)
        .push_opcode(OP_CHECKSIG)
        .into_script();
    let leaf = (wrapper, LeafVersion::TapScript);
    let leaf_hash = TapLeafHash::from_script(&leaf.0, leaf.1);
    let spend = TaprootBuilder::new()
        .add_leaf(0, leaf.0.clone())
        .unwrap()
        .finalize(secp(), owner().x_only_public_key().0)
        .unwrap();
    let mut psbt = Psbt::from_unsigned_tx(Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::absolute::LockTime::from_consensus(17),
        input: [1, 2]
            .map(|tag| TxIn {
                previous_output: OutPoint::new(
                    bitcoin::Txid::from_byte_array([tag; 32]),
                    u32::from(tag),
                ),
                sequence: Sequence::from_consensus(20 + u32::from(tag)),
                ..Default::default()
            })
            .to_vec(),
        output: vec![TxOut {
            value: Amount::from_sat(90_123),
            script_pubkey: ScriptBuf::from_bytes(vec![0x51]),
        }],
    })
    .unwrap();
    psbt.inputs[0].witness_utxo = Some(TxOut {
        value: Amount::from_sat(100_000),
        script_pubkey: ScriptBuf::new_p2tr_tweaked(spend.output_key()),
    });
    psbt.inputs[0].tap_internal_key = Some(spend.internal_key());
    psbt.inputs[0].tap_merkle_root = spend.merkle_root();
    psbt.inputs[0]
        .tap_scripts
        .insert(spend.control_block(&leaf).unwrap(), leaf);
    psbt.inputs[1].witness_utxo = Some(TxOut {
        value: Amount::from_sat(2_345),
        script_pubkey: ScriptBuf::from_bytes([&[0, 20][..], &[3; 20]].concat()),
    });
    ProgramSigningRequest {
        instance,
        input_index: 0,
        witness,
        path: ProgramSpendPath::ScriptPath(leaf_hash),
        psbt: PSBT(psbt),
    }
}

fn prevouts(request: &ProgramSigningRequest) -> Vec<TxOut> {
    request
        .psbt
        .0
        .inputs
        .iter()
        .map(|input| input.witness_utxo.clone().unwrap())
        .collect()
}

fn leaf(request: &ProgramSigningRequest) -> TapLeafHash {
    let ProgramSpendPath::ScriptPath(leaf) = request.path else {
        panic!("script-path fixture")
    };
    leaf
}

fn projection(request: &ProgramSigningRequest, annex: &[u8]) -> Vec<u8> {
    let transaction = &request.psbt.0.unsigned_tx;
    let mut bytes = transaction.version.0.to_le_bytes().to_vec();
    bytes.extend_from_slice(&transaction.lock_time.to_consensus_u32().to_le_bytes());
    bytes.extend_from_slice(&request.input_index.to_le_bytes());
    bytes.extend_from_slice(&(transaction.input.len() as u32).to_le_bytes());
    for (input, previous) in transaction.input.iter().zip(prevouts(request)) {
        input.previous_output.consensus_encode(&mut bytes).unwrap();
        bytes.extend_from_slice(&input.sequence.to_consensus_u32().to_le_bytes());
        bytes.extend_from_slice(&previous.value.to_sat().to_le_bytes());
        bytes.extend_from_slice(&(previous.script_pubkey.len() as u32).to_le_bytes());
        bytes.extend_from_slice(previous.script_pubkey.as_bytes());
    }
    bytes.extend_from_slice(&(transaction.output.len() as u32).to_le_bytes());
    for output in &transaction.output {
        bytes.extend_from_slice(&output.value.to_sat().to_le_bytes());
        bytes.extend_from_slice(&(output.script_pubkey.len() as u32).to_le_bytes());
        bytes.extend_from_slice(output.script_pubkey.as_bytes());
    }
    bytes.extend_from_slice(
        &request.psbt.0.inputs[0]
            .tap_internal_key
            .unwrap()
            .serialize(),
    );
    bytes.extend_from_slice(&(annex.len() as u32).to_le_bytes());
    bytes.extend_from_slice(annex);
    bytes
}

fn sign(
    request: &ProgramSigningRequest,
    hash_type: TapSighashType,
    selected_leaf: TapLeafHash,
) -> Vec<u8> {
    let hash = SighashCache::new(&request.psbt.0.unsigned_tx)
        .taproot_signature_hash(
            0,
            &Prevouts::All(&prevouts(request)),
            None,
            Some((selected_leaf, u32::MAX)),
            hash_type,
        )
        .unwrap();
    let mut signature = secp()
        .sign_schnorr_no_aux_rand(&Message::from_digest(hash.to_byte_array()), &owner())
        .as_ref()
        .to_vec();
    if hash_type != TapSighashType::Default {
        signature.push(hash_type as u8);
    }
    signature
}

fn denied(request: ProgramSigningRequest) {
    assert!(matches!(
        oracle().sign(request),
        Err(ProgramError::Rejected | ProgramError::Evaluation(_))
    ));
}

#[test]
fn guest_sighashes_match_bitcoin_across_serialization_boundaries() {
    let mut request = request(ScriptBuf::from_bytes(vec![0x51]), stack(&[]));
    request.psbt.0.unsigned_tx.version = bitcoin::transaction::Version(-2);
    request.psbt.0.unsigned_tx.lock_time = bitcoin::absolute::LockTime::from_consensus(0x1234_5678);
    request.psbt.0.unsigned_tx.input[0].sequence = Sequence::from_consensus(0x8765_4321);
    for script_length in [0, 252, 253, 65_535, 65_536] {
        request.psbt.0.unsigned_tx.output[0].script_pubkey =
            ScriptBuf::from_bytes(vec![0x6a; script_length]);
        for annex in [&[][..], &[0x50, 7, 8][..]] {
            let encoded = projection(&request, annex);
            let context = Context::parse_v2(&encoded).unwrap();
            for hash_type in [TapSighashType::Default, TapSighashType::All] {
                let expected = SighashCache::new(&request.psbt.0.unsigned_tx)
                    .taproot_signature_hash(
                        0,
                        &Prevouts::All(&prevouts(&request)),
                        if annex.is_empty() {
                            None
                        } else {
                            Some(Annex::new(annex).unwrap())
                        },
                        Some((leaf(&request), u32::MAX)),
                        hash_type,
                    )
                    .unwrap();
                let actual = signature_hash(
                    &context,
                    leaf(&request).as_byte_array(),
                    hash_type as u8,
                    &mut vec![0; MAX_VIEW_BYTES],
                    |bytes| Ok(sha256::Hash::hash(bytes).to_byte_array()),
                )
                .unwrap();
                assert_eq!(actual, expected.to_byte_array());
            }
        }
    }
}

#[test]
fn actual_wasm_checks_genuine_default_and_all_wrapper_leaf_signatures() {
    let script = Builder::new()
        .push_x_only_key(&owner().x_only_public_key().0)
        .push_opcode(OP_CHECKSIG)
        .into_script();
    let original = request(script.clone(), stack(&[]));
    for hash_type in [TapSighashType::Default, TapSighashType::All] {
        let signature = sign(&original, hash_type, leaf(&original));
        let mut valid = original.clone();
        valid.witness = stack(&[&signature]);
        oracle().sign(valid.clone()).unwrap();

        valid.psbt.0.unsigned_tx.output[0].value -= Amount::ONE_SAT;
        denied(valid);
    }
    let mut wrong_leaf = original;
    wrong_leaf.witness = stack(&[&sign(
        &wrong_leaf,
        TapSighashType::Default,
        TapLeafHash::from_script(&script, LeafVersion::TapScript),
    )]);
    denied(wrong_leaf);
}

#[test]
fn wasm_signature_checks_reject_invalid_encodings_and_nonempty_failures() {
    let script = Builder::new()
        .push_x_only_key(&owner().x_only_public_key().0)
        .push_opcode(OP_CHECKSIG)
        .into_script();
    let original = request(script, stack(&[]));
    let signature = sign(&original, TapSighashType::Default, leaf(&original));
    let mut invalid = signature.clone();
    invalid[17] ^= 1;
    for bytes in [
        vec![],
        vec![1],
        signature[..63].to_vec(),
        [&signature[..], &[0]].concat(),
        [&signature[..], &[2]].concat(),
        invalid,
    ] {
        let mut request = original.clone();
        request.witness = stack(&[&bytes]);
        denied(request);
    }
}

#[test]
fn script_bytes_drive_cat_execution_and_commit_the_authorizing_key() {
    let script = Builder::new()
        .push_opcode(OP_CAT)
        .push_slice(*b"abcd")
        .push_opcode(OP_EQUALVERIFY)
        .push_int(1)
        .into_script();
    let valid = request(script.clone(), stack(&[b"ab", b"cd"]));
    oracle().sign(valid.clone()).unwrap();
    let mut changed_stack = valid.clone();
    changed_stack.witness = stack(&[b"ab", b"ce"]);
    denied(changed_stack);
    let mut changed_script = script.into_bytes();
    *changed_script.last_mut().unwrap() = 0;
    let replacement = request(ScriptBuf::from_bytes(changed_script), valid.witness.clone());
    assert_ne!(valid.instance.id(), replacement.instance.id());
    assert_ne!(
        valid
            .instance
            .derive_public_key(&Xpub::from_priv(secp(), &root()))
            .unwrap(),
        replacement
            .instance
            .derive_public_key(&Xpub::from_priv(secp(), &root()))
            .unwrap()
    );
    assert_ne!(
        prevouts(&valid)[0].script_pubkey,
        prevouts(&replacement)[0].script_pubkey
    );
    denied(replacement);

    for opcode in [0x50, 0x62, 0x7f, 0xbb, 0xfe] {
        denied(request(ScriptBuf::from_bytes(vec![opcode]), stack(&[])));
    }
}
