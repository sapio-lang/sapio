use super::*;
use crate::host::{store_with_fuel, INSTANCE_FUEL};
use bitcoin::secp256k1::{Keypair, SecretKey};
use bitcoin::util::bip32::ExtendedPrivKey;
use bitcoin::Network;
use wasmer::Module;
use wasmer_middlewares::metering::{get_remaining_points, MeteringPoints};

const GUEST: &str = r#"(module
    (import "sapio_crypto_v1" "sha256"
        (func $sha (param i32 i32 i32) (result i32)))
    (import "sapio_crypto_v1" "bip32_derive"
        (func $derive (param i32 i32 i32 i32) (result i32)))
    (import "sapio_crypto_v1" "schnorr_verify"
        (func $verify (param i32 i32 i32) (result i32)))
    (memory (export "memory") 32)
    (export "sha" (func $sha))
    (export "derive" (func $derive))
    (export "verify" (func $verify))
    (func (export "guest_sha") (param i32 i32 i32) (result i32)
        local.get 0 local.get 1 local.get 2 call $sha)
    (func (export "noop")))"#;

fn guest(fuel: u64) -> (Store, Instance) {
    let mut store = store_with_fuel(fuel);
    let module = Module::new(&store, GUEST).unwrap();
    let mut imports = Imports::new();
    let env = add_crypto_imports(&mut store, &mut imports);
    let instance = Instance::new(&mut store, &module, &imports).unwrap();
    bind_crypto(&env, &mut store, &instance).unwrap();
    (store, instance)
}

fn write(store: &Store, instance: &Instance, ptr: u64, bytes: &[u8]) {
    instance
        .exports
        .get_memory("memory")
        .unwrap()
        .view(store)
        .write(ptr, bytes)
        .unwrap();
}

fn bytes<const N: usize>(store: &Store, instance: &Instance, ptr: u32) -> [u8; N] {
    read(
        &instance.exports.get_memory("memory").unwrap().view(store),
        ptr,
    )
    .unwrap()
}

fn remaining(store: &mut Store, instance: &Instance) -> u64 {
    match get_remaining_points(store, instance) {
        MeteringPoints::Remaining(points) => points,
        MeteringPoints::Exhausted => panic!("unexpected exhaustion"),
    }
}

fn root() -> ExtendedPrivKey {
    ExtendedPrivKey::new_master(Network::Bitcoin, &[0, 1, 2, 3, 4, 5, 6, 7]).unwrap()
}

#[test]
fn hashes_empty_known_and_maximum_inputs_and_allows_overlapping_output() {
    let (mut store, instance) = guest(INSTANCE_FUEL);
    let sha = instance
        .exports
        .get_typed_function::<(u32, u32, u32), i32>(&store, "guest_sha")
        .unwrap();
    assert_eq!(sha.call(&mut store, 0, 0, 128).unwrap(), 0);
    assert_eq!(
        sha256::Hash::from_inner(bytes(&store, &instance, 128)).to_string(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    write(&store, &instance, 32, b"abc");
    assert_eq!(sha.call(&mut store, 32, 3, 128).unwrap(), 0);
    assert_eq!(
        sha256::Hash::from_inner(bytes(&store, &instance, 128)).to_string(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let input = vec![0xa5; MAX_SHA256_BYTES as usize];
    write(&store, &instance, 4096, &input);
    // Output overlaps the first input chunk; hashing must finish first.
    assert_eq!(
        sha.call(&mut store, 4096, MAX_SHA256_BYTES, 4096).unwrap(),
        0
    );
    assert_eq!(
        bytes::<32>(&store, &instance, 4096),
        sha256::Hash::hash(&input).into_inner()
    );
}

#[test]
fn native_public_derivation_matches_private_derivation_at_all_depth_boundaries() {
    let (mut store, instance) = guest(INSTANCE_FUEL);
    let derive = instance
        .exports
        .get_typed_function::<(u32, u32, u32, u32), i32>(&store, "derive")
        .unwrap();
    let secp = Secp256k1::new();
    let private = root();
    let public = ExtendedPubKey::from_priv(&secp, &private);
    write(&store, &instance, 0, &public.encode());
    for indexes in [vec![], vec![0, 1, 0x7fff_ffff], vec![7; 255]] {
        let path: Vec<_> = indexes
            .iter()
            .map(|index| ChildNumber::from_normal_idx(*index).unwrap())
            .collect();
        let encoded: Vec<_> = indexes
            .iter()
            .flat_map(|index| index.to_le_bytes())
            .collect();
        write(&store, &instance, 128, &encoded);
        assert_eq!(
            derive
                .call(&mut store, 0, 128, indexes.len() as u32, 2048)
                .unwrap(),
            0
        );
        let expected = private.derive_priv(&secp, &path).unwrap();
        assert_eq!(
            bytes::<33>(&store, &instance, 2048),
            ExtendedPubKey::from_priv(&secp, &expected)
                .public_key
                .serialize()
        );
    }
    let mut deep = public;
    deep.depth = 255;
    write(&store, &instance, 0, &deep.encode());
    assert_eq!(derive.call(&mut store, 0, 128, 0, 2048).unwrap(), 0);
    assert_eq!(
        bytes::<33>(&store, &instance, 2048),
        public.public_key.serialize()
    );
    write(&store, &instance, 2048, &[0xaa; 33]);
    assert_eq!(derive.call(&mut store, 0, 128, 1, 2048).unwrap(), 1);
    assert_eq!(bytes::<33>(&store, &instance, 2048), [0xaa; 33]);
}

#[test]
fn malformed_roots_and_hardened_paths_do_not_write_outputs() {
    let (mut store, instance) = guest(INSTANCE_FUEL);
    let derive = instance
        .exports
        .get_typed_function::<(u32, u32, u32, u32), i32>(&store, "derive")
        .unwrap();
    let root = ExtendedPubKey::from_priv(&Secp256k1::new(), &root()).encode();
    write(&store, &instance, 2048, &[0x55; 33]);
    let mut bad_version = root;
    bad_version[0] ^= 1;
    let mut bad_key = root;
    bad_key[45..].fill(0);
    for invalid in [bad_version, bad_key] {
        write(&store, &instance, 0, &invalid);
        assert_eq!(derive.call(&mut store, 0, 128, 0, 2048).unwrap(), 1);
        assert_eq!(bytes::<33>(&store, &instance, 2048), [0x55; 33]);
    }
    write(&store, &instance, 0, &root);
    write(&store, &instance, 128, &0x8000_0000u32.to_le_bytes());
    assert_eq!(derive.call(&mut store, 0, 128, 1, 2048).unwrap(), 1);
    assert_eq!(bytes::<33>(&store, &instance, 2048), [0x55; 33]);
}

#[test]
fn schnorr_verification_binds_message_key_and_signature_and_rejects_invalid_data() {
    let (mut store, instance) = guest(INSTANCE_FUEL);
    let verify = instance
        .exports
        .get_typed_function::<(u32, u32, u32), i32>(&store, "verify")
        .unwrap();
    let secp = Secp256k1::new();
    let keypair = Keypair::from_secret_key(&secp, &SecretKey::from_slice(&[3; 32]).unwrap());
    let key = XOnlyPublicKey::from_keypair(&keypair).0.serialize();
    let message = [42; 32];
    let signature =
        secp.sign_schnorr_no_aux_rand(&Message::from_digest_slice(&message).unwrap(), &keypair);
    write(&store, &instance, 0, &message);
    write(&store, &instance, 32, &key);
    write(&store, &instance, 64, signature.as_ref());
    assert_eq!(verify.call(&mut store, 0, 32, 64).unwrap(), 1);
    for (ptr, original) in [
        (0, message.to_vec()),
        (32, key.to_vec()),
        (64, signature.as_ref().to_vec()),
    ] {
        let mut changed = original.clone();
        changed[0] ^= 1;
        write(&store, &instance, ptr, &changed);
        assert_eq!(verify.call(&mut store, 0, 32, 64).unwrap(), 0);
        write(&store, &instance, ptr, &original);
    }
    write(&store, &instance, 32, &[0xff; 32]);
    assert_eq!(verify.call(&mut store, 0, 32, 64).unwrap(), 0);
    write(&store, &instance, 32, &key);
    write(&store, &instance, 64, &[0xff; 64]);
    assert_eq!(verify.call(&mut store, 0, 32, 64).unwrap(), 0);
}

#[test]
fn v1_costs_are_exact_and_shared_with_instrumented_guest_code() {
    let (mut store, instance) = guest(INSTANCE_FUEL);
    let sha = instance
        .exports
        .get_typed_function::<(u32, u32, u32), i32>(&store, "sha")
        .unwrap();
    let derive = instance
        .exports
        .get_typed_function::<(u32, u32, u32, u32), i32>(&store, "derive")
        .unwrap();
    let verify = instance
        .exports
        .get_typed_function::<(u32, u32, u32), i32>(&store, "verify")
        .unwrap();
    let before = remaining(&mut store, &instance);
    assert_eq!(sha.call(&mut store, 0, 42, 2048).unwrap(), 0);
    assert_eq!(before - remaining(&mut store, &instance), 100 + 2 * 42);
    let before = remaining(&mut store, &instance);
    assert_eq!(derive.call(&mut store, 0, 128, 2, 2048).unwrap(), 1);
    assert_eq!(before - remaining(&mut store, &instance), 500 + 50_000 * 2);
    let before = remaining(&mut store, &instance);
    assert_eq!(verify.call(&mut store, 0, 32, 64).unwrap(), 0);
    assert_eq!(before - remaining(&mut store, &instance), 50_000);
    let guest_sha = instance
        .exports
        .get_typed_function::<(u32, u32, u32), i32>(&store, "guest_sha")
        .unwrap();
    let before = remaining(&mut store, &instance);
    assert_eq!(guest_sha.call(&mut store, 0, 42, 2048).unwrap(), 0);
    assert!(before - remaining(&mut store, &instance) > 100 + 2 * 42);
}

#[test]
fn fuel_is_debited_before_invalid_buffers_and_cannot_be_revived() {
    for (name, args, cost) in [
        (
            "sha",
            vec![Value::I32(-1), Value::I32(1), Value::I32(-1)],
            102,
        ),
        (
            "derive",
            vec![
                Value::I32(-1),
                Value::I32(-1),
                Value::I32(1),
                Value::I32(-1),
            ],
            50_500,
        ),
        ("verify", vec![Value::I32(-1); 3], 50_000),
    ] {
        let (mut store, instance) = guest(cost - 1);
        write(&store, &instance, 2048, &[0x77; 64]);
        let function = instance.exports.get_function(name).unwrap();
        let error = function.call(&mut store, &args).unwrap_err();
        assert!(
            error.to_string().contains("exhausted instance fuel"),
            "{error}"
        );
        assert_eq!(
            get_remaining_points(&mut store, &instance),
            MeteringPoints::Exhausted
        );
        assert_eq!(bytes::<64>(&store, &instance, 2048), [0x77; 64]);
        let sha = instance
            .exports
            .get_typed_function::<(u32, u32, u32), i32>(&store, "sha")
            .unwrap();
        assert!(sha.call(&mut store, 0, 0, 2048).is_err());
        let noop = instance
            .exports
            .get_typed_function::<(), ()>(&store, "noop")
            .unwrap();
        assert!(noop.call(&mut store).is_err());
        assert_eq!(
            get_remaining_points(&mut store, &instance),
            MeteringPoints::Exhausted
        );
    }
}

#[test]
fn every_buffer_and_oversized_request_traps_without_writing_output() {
    let (mut store, instance) = guest(INSTANCE_FUEL);
    let sha = instance
        .exports
        .get_typed_function::<(u32, u32, u32), i32>(&store, "sha")
        .unwrap();
    let derive = instance
        .exports
        .get_typed_function::<(u32, u32, u32, u32), i32>(&store, "derive")
        .unwrap();
    let verify = instance
        .exports
        .get_typed_function::<(u32, u32, u32), i32>(&store, "verify")
        .unwrap();
    let end = 32 * 65_536;
    write(&store, &instance, 2048, &[0x77; 64]);
    for (ptr, len, out) in [
        (u32::MAX, 1, 2048),
        (end, 1, 2048),
        (0, 0, end - 31),
        (0, MAX_SHA256_BYTES + 1, 2048),
    ] {
        assert!(sha.call(&mut store, ptr, len, out).is_err());
    }
    for (root, path, count, out) in [
        (end - 77, 128, 1, 2048),
        (0, end - 3, 1, 2048),
        (0, 128, 1, end - 32),
        (0, 128, 256, 2048),
    ] {
        assert!(derive.call(&mut store, root, path, count, out).is_err());
    }
    for (message, key, signature) in [(end - 31, 32, 64), (0, end - 31, 64), (0, 32, end - 63)] {
        assert!(verify.call(&mut store, message, key, signature).is_err());
    }
    assert_eq!(bytes::<64>(&store, &instance, 2048), [0x77; 64]);
    // The one-past-end pointer is valid for a zero-byte input only.
    assert_eq!(sha.call(&mut store, end, 0, 2048).unwrap(), 0);
}

#[test]
fn maximum_unsigned_lengths_have_finite_cost_and_exhaust_before_copying() {
    for (name, args) in [
        ("sha", vec![Value::I32(0), Value::I32(-1), Value::I32(2048)]),
        (
            "derive",
            vec![
                Value::I32(0),
                Value::I32(0),
                Value::I32(-1),
                Value::I32(2048),
            ],
        ),
    ] {
        let (mut store, instance) = guest(INSTANCE_FUEL);
        let error = instance
            .exports
            .get_function(name)
            .unwrap()
            .call(&mut store, &args)
            .unwrap_err();
        assert!(error.to_string().contains("exhausted instance fuel"));
        assert_eq!(
            get_remaining_points(&mut store, &instance),
            MeteringPoints::Exhausted
        );
    }
}

#[test]
fn start_calls_fail_closed_and_binding_cannot_be_replaced() {
    let mut store = store_with_fuel(INSTANCE_FUEL);
    let module = Module::new(
        &store,
        r#"(module
        (import "sapio_crypto_v1" "sha256" (func $sha (param i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func $start i32.const 0 i32.const 0 i32.const 0 call $sha drop)
        (start $start))"#,
    )
    .unwrap();
    let mut imports = Imports::new();
    let env = add_crypto_imports(&mut store, &mut imports);
    let error = Instance::new(&mut store, &module, &imports).unwrap_err();
    assert!(error
        .to_string()
        .contains("crypto environment is not initialized"));
    assert!(env.as_ref(&store).bound.is_none());

    let mut store = store_with_fuel(INSTANCE_FUEL);
    let module = Module::new(&store, GUEST).unwrap();
    let mut imports = Imports::new();
    let env = add_crypto_imports(&mut store, &mut imports);
    let instance = Instance::new(&mut store, &module, &imports).unwrap();
    bind_crypto(&env, &mut store, &instance).unwrap();
    assert!(bind_crypto(&env, &mut store, &instance).is_err());
}
