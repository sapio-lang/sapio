//! Exercise the distributed MID guest through the authenticated signing path.

use bitcoin::bip32::{Xpriv, Xpub};
use bitcoin::hashes::{sha256, Hash};
use bitcoin::hex::FromHex;
use bitcoin::opcodes::all::{OP_CHECKSIG, OP_EQUAL, OP_EQUALVERIFY};
use bitcoin::opcodes::Opcode;
use bitcoin::psbt::Psbt;
use bitcoin::script::Builder;
use bitcoin::secp256k1::{Parity, Scalar, SecretKey};
use bitcoin::taproot::{LeafVersion, TapLeafHash, TaprootBuilder};
use bitcoin::{Amount, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut};
use emulator_connect::program::{
    ProgramError, ProgramOracle, ProgramSigningRequest, ProgramSpendPath, PSBT,
};
use sapio_base::bitcoin_script::{SCRIPT_CAT_WASM, SCRIPT_MID_WASM};
use sapio_base::program::ProgramInstance;
use sapio_base::secp::secp;

const OP_TX: Opcode = bitcoin::opcodes::all::OP_RETURN_191;
const OP_TWEAKADD: Opcode = bitcoin::opcodes::all::OP_RETURN_192;

fn root() -> Xpriv {
    Xpriv::new_master(Network::Testnet, &[183; 32]).unwrap()
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

fn request(module: &[u8], script: ScriptBuf, elements: &[&[u8]]) -> ProgramSigningRequest {
    let instance = ProgramInstance::wasm_v2(module.to_vec(), script.into_bytes()).unwrap();
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
        .finalize(secp(), key)
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
        witness: stack(elements),
        path: ProgramSpendPath::ScriptPath(leaf_hash),
        psbt: PSBT(psbt),
    }
}

fn mid_request(script: ScriptBuf, elements: &[&[u8]]) -> ProgramSigningRequest {
    request(SCRIPT_MID_WASM, script, elements)
}

fn denied(request: ProgramSigningRequest) {
    assert!(matches!(
        oracle().sign(request),
        Err(ProgramError::Rejected | ProgramError::Evaluation(_))
    ));
}

#[test]
fn compiled_mid_tweak_add_matches_secp_for_both_parities_and_zero() {
    let key = root().to_keypair(secp()).x_only_public_key().0;
    let mut parities = [false; 2];
    for value in 0..16_u8 {
        let mut bytes = [0; 32];
        bytes[31] = value;
        let scalar = Scalar::from_be_bytes(bytes).unwrap();
        let (expected, parity) = key.add_tweak(secp(), &scalar).unwrap();
        let index = usize::from(parity == Parity::Odd);
        if value != 0 && parities[index] {
            continue;
        }
        parities[index] = true;
        let script = Builder::new()
            .push_opcode(OP_TWEAKADD)
            .push_slice(expected.serialize())
            .push_opcode(OP_EQUAL)
            .into_script();
        let original = mid_request(script, &[&bytes, &key.serialize()]);
        oracle().sign(original.clone()).unwrap();
        let mut incorrect = original;
        let mut changed = bytes;
        changed[31] += 1;
        incorrect.witness = stack(&[&changed, &key.serialize()]);
        denied(incorrect);
    }
    assert_eq!(parities, [true, true]);

    // The opcode returns x-only output. A second tweak must lift that output
    // with even parity, even when the first full point has odd Y.
    let first = (1..32_u8)
        .find_map(|value| {
            let mut bytes = [0; 32];
            bytes[31] = value;
            let (output, parity) = key
                .add_tweak(secp(), &Scalar::from_be_bytes(bytes).unwrap())
                .unwrap();
            (parity == Parity::Odd).then_some((bytes, output))
        })
        .unwrap();
    let mut second = [0; 32];
    second[31] = 7;
    let expected = first
        .1
        .add_tweak(secp(), &Scalar::from_be_bytes(second).unwrap())
        .unwrap()
        .0;
    let script = Builder::new()
        .push_opcode(OP_TWEAKADD)
        .push_slice(second)
        .push_opcode(bitcoin::opcodes::all::OP_SWAP)
        .push_opcode(OP_TWEAKADD)
        .push_slice(expected.serialize())
        .push_opcode(OP_EQUAL)
        .into_script();
    oracle()
        .sign(mid_request(script, &[&first.0, &key.serialize()]))
        .unwrap();
}

#[test]
fn compiled_mid_tweak_add_rejects_noncanonical_scalars_invalid_points_and_infinity() {
    let script = ScriptBuf::from_bytes(vec![0xc0]);
    let key = root().to_keypair(secp()).x_only_public_key().0.serialize();
    let mut order = Scalar::MAX.to_be_bytes();
    order[31] += 1;
    for scalar in [&order[..], &[0xff; 32][..], &[1; 31][..], &[1; 33][..]] {
        denied(mid_request(script.clone(), &[scalar, &key]));
    }
    for key in [&[0xff; 32][..], &[0; 32][..], &[1; 31][..], &[1; 33][..]] {
        denied(mid_request(script.clone(), &[&[0; 32], key]));
    }
    let mut one = [0; 32];
    one[31] = 1;
    let generator = SecretKey::from_slice(&one)
        .unwrap()
        .keypair(secp())
        .x_only_public_key()
        .0;
    denied(mid_request(
        script,
        &[&Scalar::MAX.to_be_bytes(), &generator.serialize()],
    ));
}

#[test]
fn compiled_mid_introspection_reads_authenticated_amounts_and_scripts() {
    let script = Builder::new()
        .push_int(0x8000)
        .push_opcode(OP_TX)
        .push_int(0)
        .push_opcode(OP_EQUALVERIFY)
        .push_int(0x10800)
        .push_opcode(OP_TX)
        .push_slice(100_000_u64.to_le_bytes())
        .push_opcode(OP_EQUALVERIFY)
        .push_int(0x20800)
        .push_opcode(OP_TX)
        .push_opcode(OP_EQUALVERIFY)
        .push_int(0)
        .push_int(0x4300)
        .push_opcode(OP_TX)
        .push_slice([0x51])
        .push_opcode(OP_EQUALVERIFY)
        .push_slice(90_123_u64.to_le_bytes())
        .push_opcode(OP_EQUAL)
        .into_script();
    let mut original = mid_request(script.clone(), &[]);
    let input_script = original.psbt.0.inputs[0]
        .witness_utxo
        .as_ref()
        .unwrap()
        .script_pubkey
        .clone();
    original.witness = stack(&[input_script.as_bytes()]);
    oracle().sign(original.clone()).unwrap();

    let mut changed = original.clone();
    changed.psbt.0.inputs[0]
        .witness_utxo
        .as_mut()
        .unwrap()
        .value -= Amount::ONE_SAT;
    denied(changed);
    let mut changed = original.clone();
    changed.witness = stack(&[&[0x51]]);
    denied(changed);
    let mut changed = original.clone();
    changed.psbt.0.unsigned_tx.output[0].value -= Amount::ONE_SAT;
    denied(changed);
    let mut changed = original;
    changed.psbt.0.unsigned_tx.output[0].script_pubkey = ScriptBuf::from_bytes(vec![0x52]);
    denied(changed);

    // The CAT profile retains its smaller opcode set and distinct program ID.
    denied(request(SCRIPT_CAT_WASM, script, &[input_script.as_bytes()]));
    for (selector, index) in [(1, 0), (0x10801, 0), (0x4300, 1), (0x4300, -1)] {
        let script = Builder::new()
            .push_int(index)
            .push_int(selector)
            .push_opcode(OP_TX)
            .into_script();
        denied(mid_request(script, &[]));
    }
}

#[test]
fn compiled_mid_cltv_enforces_type_boundary_and_nonfinal_sequence() {
    let script = ScriptBuf::from_bytes(vec![0xb1, 0x75, 0x51]);
    let original = mid_request(script.clone(), &[&[17]]);
    oracle().sign(original.clone()).unwrap();
    let mut old_version = original.clone();
    old_version.psbt.0.unsigned_tx.version = bitcoin::transaction::Version::ONE;
    oracle().sign(old_version).unwrap();

    let mut changed = original.clone();
    changed.psbt.0.unsigned_tx.lock_time = bitcoin::absolute::LockTime::from_consensus(16);
    denied(changed);
    let mut changed = original.clone();
    changed.psbt.0.unsigned_tx.input[0].sequence = Sequence::MAX;
    denied(changed);
    let mut changed = original;
    changed.psbt.0.unsigned_tx.lock_time = bitcoin::absolute::LockTime::from_consensus(500_000_000);
    denied(changed);
    for operand in [&[0x81][..], &[17, 0][..], &[0, 0, 0, 0, 1][..]] {
        denied(mid_request(script.clone(), &[operand]));
    }
    let mut timestamp = mid_request(script, &[&[0, 0x65, 0xcd, 0x1d]]);
    timestamp.psbt.0.unsigned_tx.lock_time =
        bitcoin::absolute::LockTime::from_consensus(500_000_000);
    oracle().sign(timestamp.clone()).unwrap();
    timestamp.psbt.0.unsigned_tx.lock_time =
        bitcoin::absolute::LockTime::from_consensus(499_999_999);
    denied(timestamp);
}

#[test]
fn compiled_mid_csfs_uses_raw_bip340_messages_and_nullfail() {
    // Reuse the official vectors pinned by the shared cryptographic runtime,
    // including empty, one-byte, 17-byte and 100-byte BIP340 messages.
    let script = ScriptBuf::from_bytes(vec![0xcc]);
    let mut counts = (0, 0);
    for line in include_str!("../../wasm/src/host/crypto/bip340_vectors.csv")
        .lines()
        .skip(1)
    {
        let fields: Vec<_> = line.split(',').collect();
        let key = Vec::<u8>::from_hex(fields[2]).unwrap();
        let message = Vec::<u8>::from_hex(fields[4]).unwrap();
        let signature = Vec::<u8>::from_hex(fields[5]).unwrap();
        let original = mid_request(script.clone(), &[&signature, &message, &key]);
        if fields[6] == "TRUE" {
            oracle().sign(original.clone()).unwrap();
            counts.0 += 1;
            let mut prehashed = original;
            prehashed.witness = stack(&[&signature, &sha256::Hash::hash(&message)[..], &key]);
            denied(prehashed);
        } else {
            denied(original);
            counts.1 += 1;
        }
    }
    assert_eq!(counts, (9, 10));

    let key = root().to_keypair(secp()).x_only_public_key().0.serialize();
    let inverse = ScriptBuf::from_bytes(vec![0xcc, 0x91]);
    oracle()
        .sign(mid_request(inverse.clone(), &[&[], b"message", &key]))
        .unwrap();
    // A bad nonempty signature terminates execution before OP_NOT can invert it.
    denied(mid_request(inverse, &[&[1; 64], b"message", &key]));
    denied(mid_request(script, &[&[1; 65], b"message", &key]));
}
