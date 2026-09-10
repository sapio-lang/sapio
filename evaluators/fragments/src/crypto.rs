use crate::{Failure, MAX_VIEW_BYTES};

#[link(wasm_import_module = "sapio_crypto_v1")]
extern "C" {
    fn sha256(pointer: u32, length: u32, output: u32) -> i32;
}

#[link(wasm_import_module = "sapio_crypto_v2")]
extern "C" {
    fn schnorr_verify(message: u32, length: u32, key: u32, signature: u32) -> i32;
    fn xonly_tweak_check(key: u32, tweak: u32, output: u32, parity: u32) -> i32;
}

pub fn hash(bytes: &[u8]) -> Result<[u8; 32], Failure> {
    if bytes.len() > MAX_VIEW_BYTES {
        return Err(Failure::InvalidEncoding);
    }
    let mut digest = [0; 32];
    let result = unsafe {
        sha256(
            bytes.as_ptr() as u32,
            bytes.len() as u32,
            digest.as_mut_ptr() as u32,
        )
    };
    match result {
        0 => Ok(digest),
        _ => Err(Failure::Crypto),
    }
}

pub fn verify(message: &[u8], key: &[u8; 32], signature: &[u8; 64]) -> Result<(), Failure> {
    if message.len() > MAX_VIEW_BYTES {
        return Err(Failure::InvalidEncoding);
    }
    let result = unsafe {
        schnorr_verify(
            message.as_ptr() as u32,
            message.len() as u32,
            key.as_ptr() as u32,
            signature.as_ptr() as u32,
        )
    };
    match result {
        1 => Ok(()),
        0 => Err(Failure::Signature),
        _ => Err(Failure::Crypto),
    }
}

pub fn tweak_check(
    key: &[u8; 32],
    tweak: &[u8; 32],
    output: &[u8; 32],
    parity: u8,
) -> Result<(), Failure> {
    let result = unsafe {
        xonly_tweak_check(
            key.as_ptr() as u32,
            tweak.as_ptr() as u32,
            output.as_ptr() as u32,
            parity as u32,
        )
    };
    match result {
        1 => Ok(()),
        0 => Err(Failure::TweakProof),
        _ => Err(Failure::Crypto),
    }
}
