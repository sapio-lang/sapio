use super::*;
use crate::host::{store_with_fuel, INSTANCE_FUEL};
use bitcoin::hex::FromHex;
use bitcoin::secp256k1::{Keypair, SecretKey};
use wasmer::Module;
use wasmer_middlewares::metering::{get_remaining_points, MeteringPoints};

const GUEST: &str = r#"(module
    (import "sapio_crypto_v1" "sha256"
        (func $sha (param i32 i32 i32) (result i32)))
    (import "sapio_crypto_v1" "schnorr_verify"
        (func $verify_v1 (param i32 i32 i32) (result i32)))
    (import "sapio_crypto_v2" "schnorr_verify"
        (func $verify (param i32 i32 i32 i32) (result i32)))
    (import "sapio_crypto_v2" "xonly_tweak_check"
        (func $tweak (param i32 i32 i32 i32) (result i32)))
    (memory (export "memory") 32)
    (export "sha" (func $sha))
    (export "verify_v1" (func $verify_v1))
    (export "verify" (func $verify))
    (export "tweak" (func $tweak))
    (func (export "noop")))"#;

fn guest(fuel: u64) -> (Store, Instance) {
    let mut store = store_with_fuel(fuel);
    let module = Module::new(&store, GUEST).unwrap();
    let mut imports = Imports::new();
    let env = add_crypto_imports_v2(&mut store, &mut imports);
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

fn remaining(store: &mut Store, instance: &Instance) -> u64 {
    match get_remaining_points(store, instance) {
        MeteringPoints::Remaining(points) => points,
        MeteringPoints::Exhausted => panic!("unexpected exhaustion"),
    }
}

fn sign(message: &[u8]) -> ([u8; 32], [u8; 64]) {
    let secp = Secp256k1::new();
    let keypair = Keypair::from_secret_key(&secp, &SecretKey::from_slice(&[3; 32]).unwrap());
    let mut signature = [0; 64];
    // SAFETY: initialized context/keypair and correctly sized, live buffers;
    // null extra parameters select libsecp256k1's default BIP340 nonce function.
    let success = unsafe {
        ffi::secp256k1_schnorrsig_sign_custom(
            secp.ctx().as_ptr(),
            signature.as_mut_ptr(),
            message.as_ptr(),
            message.len(),
            keypair.as_c_ptr(),
            std::ptr::null(),
        )
    };
    assert_eq!(success, 1);
    (
        XOnlyPublicKey::from_keypair(&keypair).0.serialize(),
        signature,
    )
}

#[test]
fn all_official_bip340_vectors_use_exact_raw_messages_and_preserve_v1() {
    // Official BIP340 code vectors, including the four arbitrary-length
    // cases added in December 2022 (0, 1, 17 and 100 message bytes).
    // Source: bitcoin/bips@200f9b26fe0a2f235a2af8b30c4be9f12f6bc9cb
    // https://github.com/bitcoin/bips/blob/200f9b26fe0a2f235a2af8b30c4be9f12f6bc9cb/bip-0340/test-vectors.csv
    // BIP340 code is available under CC0-1.0 (also BSD-2-Clause or MIT).
    let (mut store, instance) = guest(INSTANCE_FUEL);
    let verify = instance
        .exports
        .get_typed_function::<(u32, u32, u32, u32), i32>(&store, "verify")
        .unwrap();
    let verify_v1 = instance
        .exports
        .get_typed_function::<(u32, u32, u32), i32>(&store, "verify_v1")
        .unwrap();
    let mut counts = (0, 0);
    for line in include_str!("bip340_vectors.csv").lines().skip(1) {
        let fields: Vec<_> = line.split(',').collect();
        assert_eq!(fields.len(), 8);
        let key = Vec::<u8>::from_hex(fields[2]).unwrap();
        let message = Vec::<u8>::from_hex(fields[4]).unwrap();
        let signature = Vec::<u8>::from_hex(fields[5]).unwrap();
        let expected = match fields[6] {
            "TRUE" => {
                counts.0 += 1;
                1
            }
            "FALSE" => {
                counts.1 += 1;
                0
            }
            _ => panic!("invalid expected result"),
        };
        assert_eq!(key.len(), 32);
        assert_eq!(signature.len(), 64);
        write(&store, &instance, 0, &key);
        write(&store, &instance, 32, &signature);
        write(&store, &instance, 4096, &message);
        assert_eq!(
            verify
                .call(&mut store, 4096, message.len() as u32, 0, 32)
                .unwrap(),
            expected,
            "BIP340 vector {}",
            fields[0]
        );
        if message.len() == 32 {
            assert_eq!(
                verify_v1.call(&mut store, 4096, 0, 32).unwrap(),
                expected,
                "v1 BIP340 vector {}",
                fields[0]
            );
        }
        if expected == 1 {
            // An extra prehash is a different message, including for an
            // originally empty message. The v2 import must not insert one.
            write(&store, &instance, 2048, &sha256::Hash::hash(&message)[..]);
            assert_eq!(verify.call(&mut store, 2048, 32, 0, 32).unwrap(), 0);
        }
    }
    assert_eq!(counts, (9, 10));
}

#[test]
fn schnorr_accepts_maximum_message_and_binds_every_input() {
    let (mut store, instance) = guest(INSTANCE_FUEL);
    let verify = instance
        .exports
        .get_typed_function::<(u32, u32, u32, u32), i32>(&store, "verify")
        .unwrap();
    let message = vec![0xa5; MAX_SCHNORR_MESSAGE_BYTES as usize];
    let (key, signature) = sign(&message);
    write(&store, &instance, 0, &key);
    write(&store, &instance, 32, &signature);
    write(&store, &instance, 4096, &message);
    assert_eq!(
        verify
            .call(&mut store, 4096, MAX_SCHNORR_MESSAGE_BYTES, 0, 32)
            .unwrap(),
        1
    );
    assert_eq!(
        verify
            .call(&mut store, 4096, MAX_SCHNORR_MESSAGE_BYTES - 1, 0, 32)
            .unwrap(),
        0
    );
    for (pointer, original) in [
        (4096 + u64::from(MAX_SCHNORR_MESSAGE_BYTES) - 1, vec![0xa5]),
        (0, key.to_vec()),
        (32, signature.to_vec()),
    ] {
        let mut changed = original.clone();
        changed[0] ^= 1;
        write(&store, &instance, pointer, &changed);
        assert_eq!(
            verify
                .call(&mut store, 4096, MAX_SCHNORR_MESSAGE_BYTES, 0, 32)
                .unwrap(),
            0
        );
        write(&store, &instance, pointer, &original);
    }
    // Zero length may use exactly the end of memory, without dereferencing it.
    let (key, signature) = sign(&[]);
    write(&store, &instance, 0, &key);
    write(&store, &instance, 32, &signature);
    assert_eq!(verify.call(&mut store, 32 * 65_536, 0, 0, 32).unwrap(), 1);
}

#[test]
fn tweak_checks_zero_and_both_parities_and_binds_all_fields() {
    let (mut store, instance) = guest(INSTANCE_FUEL);
    let check = instance
        .exports
        .get_typed_function::<(u32, u32, u32, u32), i32>(&store, "tweak")
        .unwrap();
    let secp = Secp256k1::new();
    let secret = SecretKey::from_slice(&Scalar::ONE.to_be_bytes()).unwrap();
    let original = secret.x_only_public_key(&secp).0;
    let mut parities = [false; 2];
    for value in [0, 1, 5] {
        let mut tweak = [0; 32];
        tweak[31] = value;
        let scalar = Scalar::from_be_bytes(tweak).unwrap();
        let (tweaked, parity) = original.add_tweak(&secp, &scalar).unwrap();
        parities[parity.to_u8() as usize] = true;
        write(&store, &instance, 0, &original.serialize());
        write(&store, &instance, 32, &tweak);
        write(&store, &instance, 64, &tweaked.serialize());
        assert_eq!(
            check
                .call(&mut store, 0, 32, 64, parity.to_u8() as u32)
                .unwrap(),
            1
        );
        assert_eq!(
            check
                .call(&mut store, 0, 32, 64, (parity.to_u8() ^ 1) as u32)
                .unwrap(),
            0
        );
        for (pointer, bytes) in [
            (0, original.serialize()),
            (32, tweak),
            (64, tweaked.serialize()),
        ] {
            let mut changed = bytes;
            changed[0] ^= 1;
            write(&store, &instance, pointer, &changed);
            assert_eq!(
                check
                    .call(&mut store, 0, 32, 64, parity.to_u8() as u32)
                    .unwrap(),
                0
            );
            write(&store, &instance, pointer, &bytes);
        }
    }
    assert_eq!(parities, [true, true]);
}

#[test]
fn tweak_rejects_noncanonical_scalars_invalid_points_parity_and_infinity() {
    let (mut store, instance) = guest(INSTANCE_FUEL);
    let check = instance
        .exports
        .get_typed_function::<(u32, u32, u32, u32), i32>(&store, "tweak")
        .unwrap();
    let secp = Secp256k1::new();
    let generator = SecretKey::from_slice(&Scalar::ONE.to_be_bytes())
        .unwrap()
        .x_only_public_key(&secp)
        .0
        .serialize();
    write(&store, &instance, 0, &generator);
    write(&store, &instance, 32, &[0; 32]);
    write(&store, &instance, 64, &generator);
    assert_eq!(check.call(&mut store, 0, 32, 64, 0).unwrap(), 1);
    for parity in [2, 256, 0x8000_0000, u32::MAX] {
        assert_eq!(check.call(&mut store, 0, 32, 64, parity).unwrap(), 0);
    }
    for pointer in [0, 64] {
        write(&store, &instance, pointer, &[0xff; 32]);
        assert_eq!(check.call(&mut store, 0, 32, 64, 0).unwrap(), 0);
        write(&store, &instance, pointer, &generator);
    }
    let mut order = Scalar::MAX.to_be_bytes();
    order[31] += 1;
    for tweak in [order, [0xff; 32], Scalar::MAX.to_be_bytes()] {
        write(&store, &instance, 32, &tweak);
        // n-1 is canonical, but G + (n-1)G is infinity and must fail too.
        assert_eq!(check.call(&mut store, 0, 32, 64, 0).unwrap(), 0);
        assert_eq!(check.call(&mut store, 0, 32, 64, 1).unwrap(), 0);
    }
}

#[test]
fn v2_checks_all_buffer_bounds_before_interpreting_crypto_data() {
    let (mut store, instance) = guest(INSTANCE_FUEL);
    let verify = instance
        .exports
        .get_typed_function::<(u32, u32, u32, u32), i32>(&store, "verify")
        .unwrap();
    let tweak = instance
        .exports
        .get_typed_function::<(u32, u32, u32, u32), i32>(&store, "tweak")
        .unwrap();
    let end = 32 * 65_536;
    for (message, length, key, signature) in [
        (end, 1, 0, 32),
        (u32::MAX, 1, 0, 32),
        (u32::MAX, 0, 0, 32),
        (0, 1, end - 31, 32),
        (0, 1, 0, end - 63),
        (4096, MAX_SCHNORR_MESSAGE_BYTES + 1, 0, 32),
    ] {
        assert!(verify
            .call(&mut store, message, length, key, signature)
            .is_err());
    }
    for (original, scalar, tweaked) in [
        (end - 31, 32, 64),
        (0, end - 31, 64),
        (0, 32, end - 31),
        (u32::MAX, 32, 64),
        (0, u32::MAX, 64),
        (0, 32, u32::MAX),
    ] {
        assert!(tweak
            .call(&mut store, original, scalar, tweaked, 0)
            .is_err());
    }
}

#[test]
fn v1_and_v2_share_exact_charges_and_nonrenewable_fuel() {
    let (mut store, instance) = guest(INSTANCE_FUEL);
    let verify = instance
        .exports
        .get_typed_function::<(u32, u32, u32, u32), i32>(&store, "verify")
        .unwrap();
    let tweak = instance
        .exports
        .get_typed_function::<(u32, u32, u32, u32), i32>(&store, "tweak")
        .unwrap();
    let before = remaining(&mut store, &instance);
    assert_eq!(verify.call(&mut store, 4096, 17, 0, 32).unwrap(), 0);
    assert_eq!(before - remaining(&mut store, &instance), 50_000 + 2 * 17);
    let before = remaining(&mut store, &instance);
    assert_eq!(tweak.call(&mut store, 0, 32, 64, 0).unwrap(), 0);
    assert_eq!(before - remaining(&mut store, &instance), 50_000);
    let before = remaining(&mut store, &instance);
    assert!(verify.call(&mut store, u32::MAX, 17, 0, 32).is_err());
    assert_eq!(before - remaining(&mut store, &instance), 50_000 + 2 * 17);

    let (mut store, instance) = guest(100_000);
    let verify_v1 = instance
        .exports
        .get_typed_function::<(u32, u32, u32), i32>(&store, "verify_v1")
        .unwrap();
    let tweak = instance
        .exports
        .get_typed_function::<(u32, u32, u32, u32), i32>(&store, "tweak")
        .unwrap();
    assert_eq!(verify_v1.call(&mut store, 4096, 0, 32).unwrap(), 0);
    assert_eq!(tweak.call(&mut store, 0, 32, 64, 0).unwrap(), 0);
    assert_eq!(remaining(&mut store, &instance), 0);
    let verify = instance
        .exports
        .get_typed_function::<(u32, u32, u32, u32), i32>(&store, "verify")
        .unwrap();
    assert!(verify.call(&mut store, 4096, 0, 0, 32).is_err());
    let sha = instance
        .exports
        .get_typed_function::<(u32, u32, u32), i32>(&store, "sha")
        .unwrap();
    assert!(sha.call(&mut store, 0, 0, 1024).is_err());
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

#[test]
fn exhausted_v2_calls_debit_before_copies_and_maximum_lengths_do_not_overflow() {
    for (name, args, fuel) in [
        (
            "verify",
            [u32::MAX, 17, u32::MAX, u32::MAX],
            50_000 + 2 * 17 - 1,
        ),
        ("tweak", [u32::MAX; 4], 49_999),
        ("verify", [0, u32::MAX, 0, 32], INSTANCE_FUEL),
    ] {
        let (mut store, instance) = guest(fuel);
        let function = instance
            .exports
            .get_typed_function::<(u32, u32, u32, u32), i32>(&store, name)
            .unwrap();
        let failure = function
            .call(&mut store, args[0], args[1], args[2], args[3])
            .unwrap_err();
        assert!(failure.to_string().contains("exhausted instance fuel"));
        assert_eq!(
            get_remaining_points(&mut store, &instance),
            MeteringPoints::Exhausted
        );
    }
}

#[test]
fn v2_imports_require_explicit_host_selection_and_binding() {
    let mut store = store_with_fuel(INSTANCE_FUEL);
    let module = Module::new(&store, GUEST).unwrap();
    let mut v1_imports = Imports::new();
    add_crypto_imports(&mut store, &mut v1_imports);
    assert!(Instance::new(&mut store, &module, &v1_imports).is_err());
    let mut v2_imports = Imports::new();
    let env = add_crypto_imports_v2(&mut store, &mut v2_imports);
    let instance = Instance::new(&mut store, &module, &v2_imports).unwrap();
    let verify = instance
        .exports
        .get_typed_function::<(u32, u32, u32, u32), i32>(&store, "verify")
        .unwrap();
    let failure = verify.call(&mut store, 0, 0, 0, 32).unwrap_err();
    assert!(failure
        .to_string()
        .contains("crypto environment is not initialized"));
    bind_crypto(&env, &mut store, &instance).unwrap();
    assert_eq!(verify.call(&mut store, 0, 0, 0, 32).unwrap(), 0);
    assert!(bind_crypto(&env, &mut store, &instance).is_err());
}
