//! Bounded Bitcoin Script execution with OP_CAT over an authenticated wrapper.
//!
//! Immutable parameters are the exact Script bytes; evidence encodes its
//! initial stack as a little-endian u32 count followed by u32 lengths and data.
//! CHECKSIG authenticates the selected native wrapper leaf, rather than a
//! hypothetical on-chain CAT leaf. Its program key commits to these executable
//! bytes, while the wrapper independently enforces owner authorization and CSV.
#![no_std]

#[cfg(not(target_arch = "wasm32"))]
extern crate std;

#[cfg(target_arch = "wasm32")]
#[path = "../../v2.rs"]
mod guest;

pub mod machine;
pub mod sighash;

pub use sapio_covenant_fragments::Failure;
#[cfg(target_arch = "wasm32")]
use sapio_covenant_fragments::{Context, MAX_VIEW_BYTES};

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "sapio_crypto_v1")]
extern "C" {
    fn sha256(pointer: u32, length: u32, output: u32) -> i32;
    fn schnorr_verify(message: u32, key: u32, signature: u32) -> i32;
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "sapio_context_v2")]
extern "C" {
    fn tapleaf_hash(output: u32) -> i32;
}

#[cfg(target_arch = "wasm32")]
fn hash(bytes: &[u8]) -> Result<[u8; 32], Failure> {
    let mut output = [0; 32];
    let status = unsafe {
        sha256(
            bytes.as_ptr() as u32,
            bytes.len() as u32,
            output.as_mut_ptr() as u32,
        )
    };
    if status != 0 {
        return Err(Failure::Crypto);
    }
    Ok(output)
}

#[cfg(target_arch = "wasm32")]
struct ScriptContext<'a> {
    context: &'a Context<'a>,
    default: [u8; 32],
    all: [u8; 32],
}

#[cfg(target_arch = "wasm32")]
impl machine::Crypto for ScriptContext<'_> {
    fn hash(&self, bytes: &[u8]) -> Result<[u8; 32], Failure> {
        hash(bytes)
    }

    fn check_signature(&self, signature: &[u8], key: &[u8]) -> Result<bool, Failure> {
        if key.len() != 32 {
            return Err(Failure::Signature);
        }
        if signature.is_empty() {
            return Ok(false);
        }
        let message = match signature {
            [..] if signature.len() == 64 => &self.default,
            [.., 1] if signature.len() == 65 => &self.all,
            _ => return Err(Failure::Signature),
        };
        let status = unsafe {
            schnorr_verify(
                message.as_ptr() as u32,
                key.as_ptr() as u32,
                signature.as_ptr() as u32,
            )
        };
        match status {
            1 => Ok(true),
            // BIP342 requires every nonempty signature to verify.
            0 => Err(Failure::Signature),
            _ => Err(Failure::Crypto),
        }
    }

    fn check_sequence(&self, sequence: i64) -> Result<bool, Failure> {
        Ok(check_sequence(
            self.context.transaction_version(),
            self.context
                .sequence(self.context.input_index())
                .ok_or(Failure::InvalidEncoding)?,
            sequence,
        ))
    }
}

/// Apply BIP112's transaction-version, type and relative-sequence constraints.
pub fn check_sequence(version: i32, actual: u32, required: i64) -> bool {
    const DISABLED: u64 = 1 << 31;
    const TIME: u64 = 1 << 22;
    const MASK: u64 = TIME | 0xffff;
    if required < 0 {
        return false;
    }
    let required = required as u64;
    if required & DISABLED != 0 {
        return true;
    }
    version >= 2
        && u64::from(actual) & DISABLED == 0
        && u64::from(actual) & TIME == required & TIME
        && u64::from(actual) & MASK >= required & MASK
}

#[cfg(target_arch = "wasm32")]
static mut SCRATCH: [u8; MAX_VIEW_BYTES] = [0; MAX_VIEW_BYTES];

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn sapio_evaluate_v2(
    program_pointer: u32,
    program_length: u32,
    parameters_pointer: u32,
    parameters_length: u32,
    view_pointer: u32,
    view_length: u32,
    witness_pointer: u32,
    witness_length: u32,
) -> i32 {
    guest::evaluate(
        [
            program_pointer,
            program_length,
            parameters_pointer,
            parameters_length,
            view_pointer,
            view_length,
            witness_pointer,
            witness_length,
        ],
        |arguments| {
            if !arguments.program.is_empty() {
                return Err(Failure::InvalidEncoding);
            }
            let context = Context::parse_v2(arguments.view)?;
            // This interpreter profile retains the example's independent
            // native witness-v0 fee sponsor and unambiguous vault input.
            if context.input_index() != 0
                || context.input_count() != 2
                || !context.annex().is_empty()
            {
                return Ok(false);
            }
            context.single_vault_input()?;
            let mut leaf = [0; 32];
            if unsafe { tapleaf_hash(leaf.as_mut_ptr() as u32) } != 1 {
                return Err(Failure::InvalidEncoding);
            }
            // Fresh instances and the runtime's ban on reentrancy give this
            // scratch arena one invocation owner, separate from input bytes.
            let scratch = unsafe { &mut *core::ptr::addr_of_mut!(SCRATCH) };
            let crypto = ScriptContext {
                default: sighash::signature_hash(&context, &leaf, 0, scratch, hash)?,
                all: sighash::signature_hash(&context, &leaf, 1, scratch, hash)?,
                context: &context,
            };
            machine::evaluate(arguments.parameters, arguments.witness, &crypto)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::check_sequence;

    #[test]
    fn csv_checks_bip112_version_disable_type_and_mask() {
        assert!(check_sequence(2, 20, 20));
        assert!(check_sequence(2, 21, 20));
        assert!(!check_sequence(2, 19, 20));
        assert!(!check_sequence(1, 20, 20));
        assert!(!check_sequence(2, (1 << 31) | 20, 20));
        assert!(!check_sequence(2, (1 << 22) | 20, 20));
        assert!(!check_sequence(2, 20, (1 << 22) | 20));
        assert!(check_sequence(2, (1 << 22) | 21, (1 << 22) | 20));
        assert!(check_sequence(1, 0, 1 << 31));
        assert!(check_sequence(2, 20, (1 << 24) | 20));
        assert!(!check_sequence(2, 20, -1));
    }
}
