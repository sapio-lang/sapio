//! Bitcoin Script with CAT, interpreted by the distributed WASM-v2 guest.
//!
//! Script bytes are immutable program parameters. This never installs CAT in
//! a native Taproot leaf, where its opcode is currently OP_SUCCESS126.

use crate::program::{EmulatedProgram, ProgramError, ProgramInstance};
use bitcoin::bip32::Xpub;
use bitcoin::Script;

/// The exact distributed Script interpreter, including its CAT semantics.
pub const SCRIPT_CAT_WASM: &[u8] =
    include_bytes!("../../evaluators/artifacts/bitcoin_script_cat.wasm");
const _: () = assert!(SCRIPT_CAT_WASM.len() <= crate::program::MAX_PROGRAM_BYTES);

/// Commit an executable Script and the selected interpreter's complete bytes.
///
/// The interpreter supports a bounded, straight-line Tapscript opcode profile.
/// CHECKSIG uses the authenticated native wrapper leaf's BIP341 sighash.
pub fn emulated_script(script: &Script, root: Xpub) -> Result<EmulatedProgram, ProgramError> {
    EmulatedProgram::new(
        ProgramInstance::wasm_v2(SCRIPT_CAT_WASM.to_vec(), script.as_bytes().to_vec())?,
        root,
    )
}

/// Encode an initial Script stack as count/length-prefixed little-endian u32s.
///
/// The guest independently checks the 1,000-element stack limit, 520-byte
/// element limit, complete framing and the program protocol's evidence bound.
pub fn encode_stack(stack: &[Vec<u8>]) -> Result<Vec<u8>, &'static str> {
    if stack.len() > 1_000 {
        return Err("Script stack exceeds 1000 elements");
    }
    let size = stack
        .iter()
        .try_fold(4usize, |size, item| {
            if item.len() > 520 {
                return None;
            }
            size.checked_add(4 + item.len())
        })
        .filter(|size| *size <= 65_536)
        .ok_or("Script stack exceeds its evidence bounds")?;
    let mut bytes = Vec::with_capacity(size);
    bytes.extend_from_slice(&(stack.len() as u32).to_le_bytes());
    for item in stack {
        bytes.extend_from_slice(&(item.len() as u32).to_le_bytes());
        bytes.extend_from_slice(item);
    }
    Ok(bytes)
}
