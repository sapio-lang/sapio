//! TemplateHash composed with CSFS and explicit key-selection fragments.
#![no_std]

#[path = "../../v2.rs"]
mod guest;
use core::ptr::addr_of_mut;
use guest::Arguments;
use sapio_covenant_fragments::{
    check_sig_from_stack, Context, Failure, KnownTweak, XOnlyKey, MAX_VIEW_BYTES,
};

static mut SCRATCH: [u8; MAX_VIEW_BYTES] = [0; MAX_VIEW_BYTES];

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
        evaluate,
    )
}

fn evaluate(arguments: Arguments<'_>) -> Result<bool, Failure> {
    if !arguments.program.is_empty() {
        return Err(Failure::InvalidEncoding);
    }
    let context = Context::parse_v2(arguments.view)?;
    let (key, signature) = match arguments.parameters {
        [0, key @ ..] if key.len() == 32 => (XOnlyKey::from_slice(key)?, arguments.witness),
        [1] => (context.internal_key(), arguments.witness),
        [2] => {
            let proof = arguments
                .witness
                .get(..65)
                .ok_or(Failure::InvalidEncoding)?;
            let key = KnownTweak::from_slice(proof)?.authenticate(&context)?;
            (key, &arguments.witness[65..])
        }
        _ => return Err(Failure::InvalidEncoding),
    };
    // Scratch is independent of the immutable input arena and host state.
    let scratch = unsafe {
        core::slice::from_raw_parts_mut(addr_of_mut!(SCRATCH).cast::<u8>(), MAX_VIEW_BYTES)
    };
    let message = context.template_hash(scratch)?;
    check_sig_from_stack(message.as_bytes(), key, signature)
}
