//! Safe `no_std` guest bindings to the versioned native crypto imports.
//!
//! Available on `wasm32`. Invalid pointers, oversized host requests, and fuel
//! exhaustion trap. Cryptographic data errors return [`CryptoError`].

/// The host rejected cryptographic data or an unsupported derivation path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CryptoError;

#[link(wasm_import_module = "sapio_crypto_v1")]
extern "C" {
    #[link_name = "sha256"]
    fn host_sha256(input: *const u8, len: u32, output: *mut u8) -> i32;
    #[link_name = "bip32_derive"]
    fn host_bip32_derive(root: *const u8, path: *const u8, count: u32, output: *mut u8) -> i32;
    #[link_name = "schnorr_verify"]
    fn host_schnorr_verify(message: *const u8, key: *const u8, signature: *const u8) -> i32;
}

/// SHA256 of an input no larger than [`crate::MAX_SHA256_BYTES`].
pub fn sha256(input: &[u8]) -> Result<[u8; 32], CryptoError> {
    let mut output = [0; 32];
    // Both buffers remain live for the synchronous import call.
    let status = unsafe { host_sha256(input.as_ptr(), input.len() as u32, output.as_mut_ptr()) };
    if status == 0 {
        Ok(output)
    } else {
        Err(CryptoError)
    }
}

/// Derive a compressed public key from a 78-byte BIP32 public root.
///
/// Path entries are normal child indexes, each smaller than `2^31`. The root
/// depth plus path length must be at most 255.
pub fn bip32_derive(root: &[u8; 78], path: &[u32]) -> Result<[u8; 33], CryptoError> {
    if path.len() > crate::MAX_DERIVATION_CHILDREN as usize {
        return Err(CryptoError);
    }
    let mut encoded = [0; crate::MAX_DERIVATION_CHILDREN as usize * 4];
    for (index, child) in path.iter().enumerate() {
        encoded[index * 4..index * 4 + 4].copy_from_slice(&child.to_le_bytes());
    }
    let mut output = [0; 33];
    // Inputs and output have the exact sizes required by the import ABI.
    let status = unsafe {
        host_bip32_derive(
            root.as_ptr(),
            encoded.as_ptr(),
            path.len() as u32,
            output.as_mut_ptr(),
        )
    };
    if status == 0 {
        Ok(output)
    } else {
        Err(CryptoError)
    }
}

/// Verify a BIP340 signature against an x-only key and a 32-byte message.
pub fn schnorr_verify(message: &[u8; 32], key: &[u8; 32], signature: &[u8; 64]) -> bool {
    // All three fixed-size inputs remain live for the synchronous import.
    unsafe { host_schnorr_verify(message.as_ptr(), key.as_ptr(), signature.as_ptr()) == 1 }
}
