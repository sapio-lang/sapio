//! Safe `no_std` bindings to the version-two native verification imports.
//!
//! These require an explicitly version-two host. Invalid cryptographic data
//! returns false. Invalid buffers, oversized messages, and fuel exhaustion trap.
//! Existing hashing and derivation operations remain in [`crate::crypto`].

#[link(wasm_import_module = "sapio_crypto_v2")]
extern "C" {
    #[link_name = "schnorr_verify"]
    fn host_schnorr_verify(
        message: *const u8,
        length: u32,
        key: *const u8,
        signature: *const u8,
    ) -> i32;
    #[link_name = "xonly_tweak_check"]
    fn host_xonly_tweak_check(
        original: *const u8,
        tweak: *const u8,
        tweaked: *const u8,
        parity: u32,
    ) -> i32;
}

/// Verify BIP340 against the exact message bytes, without an extra hash.
///
/// The message may be empty and must not exceed
/// [`crate::MAX_SCHNORR_MESSAGE_BYTES`]. Signatures are exactly 64 bytes; script
/// semantics for an empty signature or a sighash suffix belong to the caller.
pub fn schnorr_verify(message: &[u8], key: &[u8; 32], signature: &[u8; 64]) -> bool {
    // wasm32 slice lengths fit u32; all inputs remain live for this call.
    unsafe {
        host_schnorr_verify(
            message.as_ptr(),
            message.len() as u32,
            key.as_ptr(),
            signature.as_ptr(),
        ) == 1
    }
}

/// Check `Q = lift_x(P) + t*G`, including the parity of Q.
///
/// `original` and `tweaked` encode the x coordinates of P and Q. `tweak` is a
/// canonical big-endian scalar below the curve order, including zero. `parity`
/// is 0 for even Q and 1 for odd Q. Infinity and malformed encodings fail.
/// This checks a scalar relation; authenticating a Taproot internal key also
/// requires checking its BIP341 commitment against the spent output.
pub fn xonly_tweak_check(
    original: &[u8; 32],
    tweak: &[u8; 32],
    tweaked: &[u8; 32],
    parity: u8,
) -> bool {
    // Fixed-size inputs remain live and the scalar parity is passed by value.
    unsafe {
        host_xonly_tweak_check(
            original.as_ptr(),
            tweak.as_ptr(),
            tweaked.as_ptr(),
            u32::from(parity),
        ) == 1
    }
}
