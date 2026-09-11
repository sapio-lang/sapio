//! Versioned native cryptography, charged to the calling instance's fuel.

use crate::{
    CRYPTO_NAMESPACE, CRYPTO_NAMESPACE_V2, MAX_DERIVATION_CHILDREN, MAX_SCHNORR_MESSAGE_BYTES,
    MAX_SHA256_BYTES,
};
use bitcoin::bip32::{ChildNumber, Xpub};
use bitcoin::hashes::{sha256, Hash, HashEngine};
use bitcoin::secp256k1::ffi::{self, CPtr};
use bitcoin::secp256k1::{schnorr, Message, Parity, Scalar, Secp256k1, VerifyOnly, XOnlyPublicKey};
use std::sync::OnceLock;
use wasmer::{
    AsStoreMut, Function, FunctionEnv, FunctionEnvMut, Global, Imports, Instance, Memory,
    MemoryView, RuntimeError, Store, Value,
};

/// Fixed v1 cost before hashing any bytes.
pub const SHA256_BASE_FUEL: u64 = 100;
/// Fixed v1 cost for each input byte hashed.
pub const SHA256_BYTE_FUEL: u64 = 2;
/// Fixed v1 cost before decoding a BIP32 public root.
pub const BIP32_BASE_FUEL: u64 = 500;
/// Fixed v1 cost for each requested normal BIP32 child.
pub const BIP32_CHILD_FUEL: u64 = 50_000;
/// Fixed v1 cost for each Schnorr verification, including invalid data.
pub const SCHNORR_VERIFY_FUEL: u64 = 50_000;
/// Fixed v2 cost for each raw-message Schnorr verification.
pub const SCHNORR_VERIFY_V2_BASE_FUEL: u64 = 50_000;
/// Additional v2 cost for each raw message byte verified.
pub const SCHNORR_VERIFY_V2_BYTE_FUEL: u64 = 2;
/// Fixed v2 cost for each x-only key tweak check, including invalid data.
pub const XONLY_TWEAK_CHECK_FUEL: u64 = 50_000;

/// Crypto imports are unusable until bound to their instantiated guest.
#[derive(Default)]
pub struct CryptoEnvironment {
    bound: Option<BoundCrypto>,
}

#[derive(Clone)]
struct BoundCrypto {
    memory: Memory,
    remaining: Global,
    exhausted: Global,
}

fn error(message: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::new(message.to_string())
}

/// Add only the three v1 cryptographic imports to an existing import map.
///
/// Call [`bind_crypto`] after instantiation and before executing guest exports.
/// A crypto call from a WASM start function traps before accessing memory.
pub fn add_crypto_imports(
    store: &mut Store,
    imports: &mut Imports,
) -> FunctionEnv<CryptoEnvironment> {
    let env = FunctionEnv::new(store, CryptoEnvironment::default());
    imports.define(
        CRYPTO_NAMESPACE,
        "sha256",
        Function::new_typed_with_env(store, &env, sha256_import),
    );
    imports.define(
        CRYPTO_NAMESPACE,
        "bip32_derive",
        Function::new_typed_with_env(store, &env, bip32_derive_import),
    );
    imports.define(
        CRYPTO_NAMESPACE,
        "schnorr_verify",
        Function::new_typed_with_env(store, &env, schnorr_verify_import),
    );
    env
}

/// Add the unchanged v1 imports and two v2 verification imports.
///
/// Both namespaces share one environment and the instance's existing fuel
/// globals. Bind the returned environment once using [`bind_crypto`]. Hosts
/// must select this extension explicitly; [`add_crypto_imports`] stays v1-only.
pub fn add_crypto_imports_v2(
    store: &mut Store,
    imports: &mut Imports,
) -> FunctionEnv<CryptoEnvironment> {
    let env = add_crypto_imports(store, imports);
    imports.define(
        CRYPTO_NAMESPACE_V2,
        "schnorr_verify",
        Function::new_typed_with_env(store, &env, schnorr_verify_v2_import),
    );
    imports.define(
        CRYPTO_NAMESPACE_V2,
        "xonly_tweak_check",
        Function::new_typed_with_env(store, &env, xonly_tweak_check_import),
    );
    env
}

/// Bind the guest's exported memory and injected fuel globals exactly once.
///
/// The instance must have been compiled by a store from this crate. Only
/// handles are retained, so the environment does not create an instance cycle.
pub fn bind_crypto(
    env: &FunctionEnv<CryptoEnvironment>,
    store: &mut Store,
    instance: &Instance,
) -> Result<(), RuntimeError> {
    if env.as_ref(store).bound.is_some() {
        return Err(error("WASM crypto environment is already bound"));
    }
    let bound = BoundCrypto {
        memory: instance
            .exports
            .get_memory("memory")
            .map_err(error)?
            .clone(),
        remaining: instance
            .exports
            .get_global("wasmer_metering_remaining_points")
            .map_err(error)?
            .clone(),
        exhausted: instance
            .exports
            .get_global("wasmer_metering_points_exhausted")
            .map_err(error)?
            .clone(),
    };
    env.as_mut(store).bound = Some(bound);
    Ok(())
}

impl BoundCrypto {
    fn from_env(env: &FunctionEnvMut<'_, CryptoEnvironment>) -> Result<Self, RuntimeError> {
        env.data()
            .bound
            .clone()
            .ok_or_else(|| error("WASM crypto environment is not initialized"))
    }

    fn charge(&self, store: &mut impl AsStoreMut, cost: u64) -> Result<(), RuntimeError> {
        // Do not use set_remaining_points: it clears the exhaustion flag.
        // Host calls and instrumented WASM debit these same appended globals.
        let Value::I32(exhausted) = self.exhausted.get(store) else {
            return Err(error("WASM crypto exhausted global has the wrong type"));
        };
        let Value::I64(remaining) = self.remaining.get(store) else {
            return Err(error("WASM crypto remaining global has the wrong type"));
        };
        if exhausted != 0 || (remaining as u64) < cost {
            self.exhausted.set(store, Value::I32(1))?;
            self.remaining.set(store, Value::I64(0))?;
            return Err(error("WASM crypto exhausted instance fuel"));
        }
        self.remaining
            .set(store, Value::I64(((remaining as u64) - cost) as i64))
    }
}

fn check_range(memory: &MemoryView<'_>, ptr: u32, len: u32) -> Result<(), RuntimeError> {
    if u64::from(ptr) + u64::from(len) > memory.data_size() {
        return Err(error("WASM crypto buffer is outside guest memory"));
    }
    Ok(())
}

fn read<const N: usize>(memory: &MemoryView<'_>, ptr: u32) -> Result<[u8; N], RuntimeError> {
    check_range(memory, ptr, N as u32)?;
    let mut output = [0; N];
    memory.read(u64::from(ptr), &mut output).map_err(error)?;
    Ok(output)
}

fn secp() -> &'static Secp256k1<VerifyOnly> {
    static SECP: OnceLock<Secp256k1<VerifyOnly>> = OnceLock::new();
    SECP.get_or_init(Secp256k1::verification_only)
}

fn sha256_import(
    mut env: FunctionEnvMut<'_, CryptoEnvironment>,
    ptr: u32,
    len: u32,
    out: u32,
) -> Result<i32, RuntimeError> {
    let bound = BoundCrypto::from_env(&env)?;
    bound.charge(
        &mut env,
        SHA256_BASE_FUEL + SHA256_BYTE_FUEL * u64::from(len),
    )?;
    if len > MAX_SHA256_BYTES {
        return Err(error("WASM SHA256 input exceeds 1 MiB"));
    }
    let memory = bound.memory.view(&env);
    check_range(&memory, ptr, len)?;
    check_range(&memory, out, 32)?;
    // Read in bounded chunks; all input is consumed before writing the output,
    // including when the caller's input and output ranges overlap.
    let mut engine = sha256::Hash::engine();
    let mut chunk = [0; 4096];
    let mut offset = 0;
    while offset < len {
        let count = (len - offset).min(chunk.len() as u32) as usize;
        memory
            .read(u64::from(ptr) + u64::from(offset), &mut chunk[..count])
            .map_err(error)?;
        engine.input(&chunk[..count]);
        offset += count as u32;
    }
    memory
        .write(u64::from(out), &sha256::Hash::from_engine(engine)[..])
        .map_err(error)?;
    Ok(0)
}

fn bip32_derive_import(
    mut env: FunctionEnvMut<'_, CryptoEnvironment>,
    root: u32,
    path: u32,
    count: u32,
    out: u32,
) -> Result<i32, RuntimeError> {
    let bound = BoundCrypto::from_env(&env)?;
    bound.charge(
        &mut env,
        BIP32_BASE_FUEL + BIP32_CHILD_FUEL * u64::from(count),
    )?;
    if count > MAX_DERIVATION_CHILDREN {
        return Err(error("WASM BIP32 path exceeds 255 children"));
    }
    let memory = bound.memory.view(&env);
    let encoded_root = read::<78>(&memory, root)?;
    check_range(&memory, path, count * 4)?;
    check_range(&memory, out, 33)?;
    let mut encoded_path = [0; MAX_DERIVATION_CHILDREN as usize * 4];
    memory
        .read(u64::from(path), &mut encoded_path[..count as usize * 4])
        .map_err(error)?;
    let Ok(root) = Xpub::decode(&encoded_root) else {
        return Ok(1);
    };
    if u32::from(root.depth) + count > 255 {
        return Ok(1);
    }
    let mut children = Vec::with_capacity(count as usize);
    for bytes in encoded_path[..count as usize * 4].as_chunks::<4>().0 {
        let index = u32::from_le_bytes(*bytes);
        let Ok(child) = ChildNumber::from_normal_idx(index) else {
            return Ok(1);
        };
        children.push(child);
    }
    let Ok(derived) = root.derive_pub(secp(), &children) else {
        return Ok(1);
    };
    memory
        .write(u64::from(out), &derived.public_key.serialize())
        .map_err(error)?;
    Ok(0)
}

fn schnorr_verify_import(
    mut env: FunctionEnvMut<'_, CryptoEnvironment>,
    message: u32,
    key: u32,
    signature: u32,
) -> Result<i32, RuntimeError> {
    let bound = BoundCrypto::from_env(&env)?;
    bound.charge(&mut env, SCHNORR_VERIFY_FUEL)?;
    let memory = bound.memory.view(&env);
    let message = read::<32>(&memory, message)?;
    let key = read::<32>(&memory, key)?;
    let signature = read::<64>(&memory, signature)?;
    let Ok(key) = XOnlyPublicKey::from_slice(&key) else {
        return Ok(0);
    };
    let Ok(signature) = schnorr::Signature::from_slice(&signature) else {
        return Ok(0);
    };
    let message = Message::from_digest_slice(&message).expect("32-byte message");
    Ok(i32::from(
        secp().verify_schnorr(&signature, &message, &key).is_ok(),
    ))
}

fn schnorr_verify_v2_import(
    mut env: FunctionEnvMut<'_, CryptoEnvironment>,
    message: u32,
    length: u32,
    key: u32,
    signature: u32,
) -> Result<i32, RuntimeError> {
    let bound = BoundCrypto::from_env(&env)?;
    bound.charge(
        &mut env,
        SCHNORR_VERIFY_V2_BASE_FUEL + SCHNORR_VERIFY_V2_BYTE_FUEL * u64::from(length),
    )?;
    if length > MAX_SCHNORR_MESSAGE_BYTES {
        return Err(error("WASM Schnorr message exceeds 1 MiB"));
    }
    let memory = bound.memory.view(&env);
    check_range(&memory, message, length)?;
    let key = read::<32>(&memory, key)?;
    let signature = read::<64>(&memory, signature)?;
    let Ok(key) = XOnlyPublicKey::from_slice(&key) else {
        return Ok(0);
    };
    let mut bytes = vec![0; length as usize];
    memory.read(u64::from(message), &mut bytes).map_err(error)?;
    // SAFETY: the context and parsed key are initialized libsecp256k1 objects;
    // signature contains exactly 64 bytes, and bytes lives for this synchronous
    // call with the stated length (including zero). The C API accepts raw
    // messages; the pinned safe Rust wrapper only accepts 32-byte Message.
    // Prehashing here would change BIP340/CSFS verification semantics.
    let valid = unsafe {
        ffi::secp256k1_schnorrsig_verify(
            secp().ctx().as_ptr(),
            signature.as_ptr(),
            bytes.as_ptr(),
            bytes.len(),
            key.as_c_ptr(),
        )
    };
    Ok(i32::from(valid == 1))
}

fn xonly_tweak_check_import(
    mut env: FunctionEnvMut<'_, CryptoEnvironment>,
    original: u32,
    tweak: u32,
    tweaked: u32,
    parity: u32,
) -> Result<i32, RuntimeError> {
    let bound = BoundCrypto::from_env(&env)?;
    bound.charge(&mut env, XONLY_TWEAK_CHECK_FUEL)?;
    let memory = bound.memory.view(&env);
    let original = read::<32>(&memory, original)?;
    let tweak = read::<32>(&memory, tweak)?;
    let tweaked = read::<32>(&memory, tweaked)?;
    let parity = match parity {
        0 => Parity::Even,
        1 => Parity::Odd,
        _ => return Ok(0),
    };
    let (Ok(original), Ok(tweaked), Ok(tweak)) = (
        XOnlyPublicKey::from_slice(&original),
        XOnlyPublicKey::from_slice(&tweaked),
        Scalar::from_be_bytes(tweak),
    ) else {
        return Ok(0);
    };
    Ok(i32::from(original.tweak_add_check(
        secp(),
        &tweaked,
        parity,
        tweak,
    )))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_v2;
