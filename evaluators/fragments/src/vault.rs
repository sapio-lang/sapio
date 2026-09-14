//! Leaf-update and recovery primitives for a single-input BIP345 profile.
//!
//! The source leaf is authenticated by the signing context import. Merkle
//! evidence cannot select another leaf from the same tree. The profile keeps
//! the full vault principal and requires witness-v0 inputs for external fees.

use crate::{crypto, Context, Failure, XOnlyKey};

const CTV_WASM: &[u8] = include_bytes!("../../artifacts/ctv.wasm");

#[link(wasm_import_module = "sapio_context_v2")]
extern "C" {
    fn tapleaf_hash(output: u32) -> i32;
}

/// Maximum tapscript byte length accepted by this profile.
pub const MAX_LEAF_BYTES: usize = 10_000;

/// Derive the distributed inline-CTV program key under the committed root.
pub fn ctv_key(
    root: &[u8; 78],
    expected: &[u8; 32],
    scratch: &mut [u8],
) -> Result<XOnlyKey, Failure> {
    let tag = crypto::hash(b"Sapio/Emulation/Program/v1")?;
    let mut writer = Writer::new(scratch);
    writer.put(&tag)?;
    writer.put(&tag)?;
    writer.put(&[0; 32])?;
    writer.put(&(CTV_WASM.len() as u32).to_le_bytes())?;
    writer.put(CTV_WASM)?;
    writer.put(&32_u32.to_le_bytes())?;
    writer.put(expected)?;
    let id = crypto::hash(writer.written())?;
    let mut path = [0; 10];
    path[0] = 0x5341_5049;
    for (index, bytes) in id.chunks_exact(4).enumerate() {
        let word = u32::from_be_bytes(bytes.try_into().map_err(|_| Failure::InvalidEncoding)?);
        path[index + 1] = word & 0x7fff_ffff;
        path[9] |= (word >> 31) << index;
    }
    XOnlyKey::from_slice(&crypto::derive_key(root, &path)?)
}

/// Encode `and_v(v:pk(key),older(delay))` with a positive block delay.
pub fn delayed_key_leaf<'a>(
    key: XOnlyKey,
    delay: u16,
    output: &'a mut [u8; 40],
) -> Result<&'a [u8], Failure> {
    if delay == 0 {
        return Err(Failure::InvalidEncoding);
    }
    let mut writer = Writer::new(output);
    writer.put(&[32])?;
    writer.put(key.as_bytes())?;
    writer.put(&[0xad])?; // CHECKSIGVERIFY
    if delay <= 16 {
        writer.put(&[0x50 + delay as u8])?;
    } else {
        let bytes = delay.to_le_bytes();
        let width = if delay <= 0x7f {
            1
        } else if delay <= 0x7fff {
            2
        } else {
            3
        };
        writer.put(&[width])?;
        writer.put(&bytes[..usize::from(width.min(2))])?;
        if width == 3 {
            writer.put(&[0])?;
        }
    }
    writer.put(&[0xb2])?; // CHECKSEQUENCEVERIFY
    let length = writer.used;
    Ok(&output[..length])
}

/// Verify an authenticated leaf replacement while retaining its internal key
/// and every Merkle sibling, as required by the historical BIP345 operation.
pub fn verify_leaf_replacement(
    context: &Context<'_>,
    source_script: &[u8],
    control: &[u8],
    replacement_script: &[u8],
    replacement_key: XOnlyKey,
    scratch: &mut [u8],
) -> Result<(), Failure> {
    if source_script.len() > MAX_LEAF_BYTES
        || replacement_script.len() > MAX_LEAF_BYTES
        || control.len() < 33
        || control.len() > 33 + 32 * 128
        || (control.len() - 33) % 32 != 0
        || control[0] & 0xfe != 0xc0
        || &control[1..33] != context.internal_key().as_bytes()
    {
        return Err(Failure::InvalidEncoding);
    }
    let source = leaf_hash(source_script, scratch)?;
    let mut selected = [0; 32];
    if unsafe { tapleaf_hash(selected.as_mut_ptr() as u32) } != 1 || source != selected {
        return Err(Failure::TweakProof);
    }
    let original_root = merkle_root(source, &control[33..], scratch)?;
    let original_tweak = tap_tweak(context.internal_key(), original_root, scratch)?;
    crypto::tweak_check(
        context.internal_key().as_bytes(),
        &original_tweak,
        context.output_key().as_bytes(),
        control[0] & 1,
    )?;

    let replacement = leaf_hash(replacement_script, scratch)?;
    let replacement_root = merkle_root(replacement, &control[33..], scratch)?;
    let replacement_tweak = tap_tweak(context.internal_key(), replacement_root, scratch)?;
    match crypto::tweak_check(
        context.internal_key().as_bytes(),
        &replacement_tweak,
        replacement_key.as_bytes(),
        0,
    ) {
        Ok(()) => Ok(()),
        Err(Failure::TweakProof) => crypto::tweak_check(
            context.internal_key().as_bytes(),
            &replacement_tweak,
            replacement_key.as_bytes(),
            1,
        ),
        Err(error) => Err(error),
    }
}

/// The BIP345 tagged commitment to a recovery output's serialized script.
pub fn recovery_hash(script: &[u8], scratch: &mut [u8]) -> Result<[u8; 32], Failure> {
    let tag = crypto::hash(b"VaultRecoverySPK")?;
    let mut writer = Writer::new(scratch);
    writer.put(&tag)?;
    writer.put(&tag)?;
    writer.compact_size(script.len())?;
    writer.put(script)?;
    crypto::hash(writer.written())
}

fn leaf_hash(script: &[u8], scratch: &mut [u8]) -> Result<[u8; 32], Failure> {
    let tag = crypto::hash(b"TapLeaf")?;
    let mut writer = Writer::new(scratch);
    writer.put(&tag)?;
    writer.put(&tag)?;
    writer.put(&[0xc0])?;
    writer.compact_size(script.len())?;
    writer.put(script)?;
    crypto::hash(writer.written())
}

fn merkle_root(
    mut hash: [u8; 32],
    siblings: &[u8],
    scratch: &mut [u8],
) -> Result<[u8; 32], Failure> {
    let tag = crypto::hash(b"TapBranch")?;
    for sibling in siblings.chunks_exact(32) {
        let mut writer = Writer::new(scratch);
        writer.put(&tag)?;
        writer.put(&tag)?;
        if hash.as_slice() < sibling {
            writer.put(&hash)?;
            writer.put(sibling)?;
        } else {
            writer.put(sibling)?;
            writer.put(&hash)?;
        }
        hash = crypto::hash(writer.written())?;
    }
    Ok(hash)
}

fn tap_tweak(key: XOnlyKey, root: [u8; 32], scratch: &mut [u8]) -> Result<[u8; 32], Failure> {
    let tag = crypto::hash(b"TapTweak")?;
    let mut writer = Writer::new(scratch);
    writer.put(&tag)?;
    writer.put(&tag)?;
    writer.put(key.as_bytes())?;
    writer.put(&root)?;
    crypto::hash(writer.written())
}

struct Writer<'a> {
    bytes: &'a mut [u8],
    used: usize,
}

impl<'a> Writer<'a> {
    fn new(bytes: &'a mut [u8]) -> Self {
        Self { bytes, used: 0 }
    }
    fn put(&mut self, bytes: &[u8]) -> Result<(), Failure> {
        let end = self
            .used
            .checked_add(bytes.len())
            .ok_or(Failure::ScratchTooSmall)?;
        self.bytes
            .get_mut(self.used..end)
            .ok_or(Failure::ScratchTooSmall)?
            .copy_from_slice(bytes);
        self.used = end;
        Ok(())
    }
    fn compact_size(&mut self, length: usize) -> Result<(), Failure> {
        if length < 253 {
            self.put(&[length as u8])
        } else if length <= u16::MAX as usize {
            self.put(&[253])?;
            self.put(&(length as u16).to_le_bytes())
        } else {
            let length = u32::try_from(length).map_err(|_| Failure::InvalidEncoding)?;
            self.put(&[254])?;
            self.put(&length.to_le_bytes())
        }
    }
    fn written(&self) -> &[u8] {
        &self.bytes[..self.used]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delays_use_minimal_positive_script_numbers() {
        let key = XOnlyKey::from_slice(&[2; 32]).unwrap();
        let mut bytes = [0; 40];
        for (delay, suffix) in [
            (1, &[0x51, 0xb2][..]),
            (16, &[0x60, 0xb2]),
            (17, &[1, 17, 0xb2]),
            (127, &[1, 127, 0xb2]),
            (128, &[2, 128, 0, 0xb2]),
            (32767, &[2, 255, 127, 0xb2]),
            (32768, &[3, 0, 128, 0, 0xb2]),
            (65535, &[3, 255, 255, 0, 0xb2]),
        ] {
            let script = delayed_key_leaf(key, delay, &mut bytes).unwrap();
            assert_eq!(&script[..34], &[&[32][..], &[2; 32], &[0xad]].concat());
            assert_eq!(&script[34..], suffix);
        }
        assert_eq!(
            delayed_key_leaf(key, 0, &mut bytes),
            Err(Failure::InvalidEncoding)
        );
    }
}
