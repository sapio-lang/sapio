//! A BIP446 equality predicate built by composing the fragment SDK.
#![no_std]

#[path = "../../v2.rs"]
mod guest;
use core::ptr::addr_of_mut;
use guest::Arguments;
use sapio_covenant_fragments::{Context, Failure, MAX_VIEW_BYTES};

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
    if !arguments.program.is_empty()
        || !arguments.witness.is_empty()
        || arguments.parameters.len() != 32
    {
        return Err(Failure::InvalidEncoding);
    }
    let context = Context::parse_v2(arguments.view)?;
    // This synchronous instance invocation exclusively owns its scratch.
    let scratch = unsafe {
        core::slice::from_raw_parts_mut(addr_of_mut!(SCRATCH).cast::<u8>(), MAX_VIEW_BYTES)
    };
    Ok(context.template_hash(scratch)?.as_bytes() == arguments.parameters)
}
