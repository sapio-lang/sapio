//! Exact payment semantics interpreted by a bounded WASM program.
#![no_std]

#[path = "../../common.rs"]
mod guest;
use guest::{Arguments, Reader};

#[no_mangle]
pub extern "C" fn sapio_evaluate_v1(
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

fn evaluate(arguments: Arguments<'_>) -> Option<bool> {
    let Arguments {
        program,
        parameters,
        view,
        witness,
    } = arguments;
    if program != b"pay-at-least/v1" {
        return None;
    }
    let mut params = Reader::new(parameters);
    let minimum = u64::from_le_bytes(params.take(8)?.try_into().ok()?);
    let script_length = params.u32()?;
    let recipient = params.take(script_length as usize)?;
    if !params.is_finished() {
        return None;
    }
    let selected_output = u32::from_le_bytes(witness.try_into().ok()?);
    let mut reader = Reader::new(view);
    reader.take(8)?;
    let selected_input = reader.u32()?;
    let inputs = reader.u32()?;
    if selected_input >= inputs {
        return None;
    }
    for _ in 0..inputs {
        reader.take(48)?;
        let length = reader.u32()?;
        reader.take(length as usize)?;
    }
    let outputs = reader.u32()?;
    let mut accepted = false;
    for index in 0..outputs {
        let amount = u64::from_le_bytes(reader.take(8)?.try_into().ok()?);
        let length = reader.u32()?;
        let script = reader.take(length as usize)?;
        if index == selected_output {
            accepted = amount >= minimum && script == recipient;
        }
    }
    reader.is_finished().then_some(accepted)
}
