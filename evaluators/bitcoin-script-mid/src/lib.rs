//! Bounded execution of the LayerTree MID opcode profile.
//!
//! Parameters are executable Script bytes. All introspection reads the signed
//! transaction projection; neither roots nor transition rules live in this VM.
#![no_std]

#[cfg(not(target_arch = "wasm32"))]
extern crate std;

#[cfg(target_arch = "wasm32")]
#[path = "../../v2.rs"]
mod guest;

// The two separately distributed guests share one source implementation.
// Importing the CAT cdylib would also link its guest entrypoint and panic ABI.
#[path = "../../bitcoin-script-cat/src/machine.rs"]
pub mod machine;
#[path = "../../bitcoin-script-cat/src/sighash.rs"]
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
#[link(wasm_import_module = "sapio_crypto_v2")]
extern "C" {
    #[link_name = "schnorr_verify"]
    fn verify_data(message: u32, length: u32, key: u32, signature: u32) -> i32;
    fn xonly_tweak_add(original: u32, tweak: u32, output: u32) -> i32;
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
    (status == 0).then_some(output).ok_or(Failure::Crypto)
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
            _ if signature.len() == 64 => &self.default,
            [.., 1] if signature.len() == 65 => &self.all,
            _ => return Err(Failure::Signature),
        };
        match unsafe {
            schnorr_verify(
                message.as_ptr() as u32,
                key.as_ptr() as u32,
                signature.as_ptr() as u32,
            )
        } {
            1 => Ok(true),
            0 => Err(Failure::Signature),
            _ => Err(Failure::Crypto),
        }
    }

    fn check_sequence(&self, operand: i64) -> Result<bool, Failure> {
        Ok(machine::check_sequence(
            self.context.transaction_version(),
            self.context
                .sequence(self.context.input_index())
                .ok_or(Failure::InvalidEncoding)?,
            operand,
        ))
    }

    fn check_lock_time(&self, operand: i64) -> Result<bool, Failure> {
        Ok(check_lock_time(
            self.context.lock_time(),
            self.context
                .sequence(self.context.input_index())
                .ok_or(Failure::InvalidEncoding)?,
            operand,
        ))
    }

    fn check_data_signature(
        &self,
        signature: &[u8],
        message: &[u8],
        key: &[u8],
    ) -> Result<bool, Failure> {
        if key.len() != 32 {
            return Err(Failure::Signature);
        }
        if signature.is_empty() {
            return Ok(false);
        }
        if signature.len() != 64 {
            return Err(Failure::Signature);
        }
        match unsafe {
            verify_data(
                message.as_ptr() as u32,
                message.len() as u32,
                key.as_ptr() as u32,
                signature.as_ptr() as u32,
            )
        } {
            1 => Ok(true),
            0 => Err(Failure::Signature),
            _ => Err(Failure::Crypto),
        }
    }

    fn tweak_add(&self, tweak: &[u8; 32], key: &[u8; 32]) -> Result<[u8; 32], Failure> {
        let mut output = [0; 32];
        match unsafe {
            xonly_tweak_add(
                key.as_ptr() as u32,
                tweak.as_ptr() as u32,
                output.as_mut_ptr() as u32,
            )
        } {
            1 => Ok(output),
            0 => Err(Failure::TweakProof),
            _ => Err(Failure::Crypto),
        }
    }

    fn transaction(
        &self,
        selector: u32,
        index: Option<u32>,
        output: &mut [u8; 520],
    ) -> Result<(usize, Option<u64>), Failure> {
        let (bytes, amount) = match (selector, index) {
            (0x8000, None) => return Ok((0, None)), // selected input is zero
            (0x10800, None) => {
                let (amount, _) = self.context.input(0).ok_or(Failure::InvalidEncoding)?;
                output[..8].copy_from_slice(&amount.to_le_bytes());
                return Ok((8, None));
            }
            (0x20800, None) => (
                self.context.input(0).ok_or(Failure::InvalidEncoding)?.1,
                None,
            ),
            (0x4300, Some(index)) => {
                let (amount, script) =
                    self.context.output(index).ok_or(Failure::InvalidEncoding)?;
                (script, Some(amount))
            }
            _ => return Err(Failure::InvalidEncoding),
        };
        output
            .get_mut(..bytes.len())
            .ok_or(Failure::InvalidEncoding)?
            .copy_from_slice(bytes);
        Ok((bytes.len(), amount))
    }
}

/// BIP65 lock-time comparison; Bitcoin enforces the actual chain height/time.
pub fn check_lock_time(actual: u32, sequence: u32, required: i64) -> bool {
    const THRESHOLD: u32 = 500_000_000;
    let Ok(required) = u32::try_from(required) else {
        return false;
    };
    sequence != u32::MAX && (actual < THRESHOLD) == (required < THRESHOLD) && actual >= required
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
            if context.input_index() != 0
                || context.input_count() < 2
                || !context.annex().is_empty()
            {
                return Ok(false);
            }
            // Only one covenant principal may be claimed. Other inputs pay for
            // admission and fees and cannot be independent Taproot covenants.
            context.single_vault_input()?;
            let mut leaf = [0; 32];
            if unsafe { tapleaf_hash(leaf.as_mut_ptr() as u32) } != 1 {
                return Err(Failure::InvalidEncoding);
            }
            let scratch = unsafe { &mut *core::ptr::addr_of_mut!(SCRATCH) };
            let crypto = ScriptContext {
                default: sighash::signature_hash(&context, &leaf, 0, scratch, hash)?,
                all: sighash::signature_hash(&context, &leaf, 1, scratch, hash)?,
                context: &context,
            };
            machine::evaluate_mid(arguments.parameters, arguments.witness, &crypto)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::check_lock_time;
    #[test]
    fn lock_time_respects_height_time_sequence_and_range() {
        assert!(check_lock_time(100, 0xffff_fffe, 100));
        assert!(check_lock_time(101, 0, 100));
        assert!(!check_lock_time(99, 0, 100));
        assert!(!check_lock_time(100, u32::MAX, 100));
        assert!(!check_lock_time(500_000_000, 0, 100));
        assert!(check_lock_time(500_000_001, 0, 500_000_000));
        assert!(!check_lock_time(100, 0, -1));
        assert!(!check_lock_time(100, 0, i64::MAX));
    }
}
