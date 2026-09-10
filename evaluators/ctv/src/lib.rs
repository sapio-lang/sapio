//! CTV evaluation over the signed transaction view, with no Bitcoin library.
#![no_std]

#[path = "../../common.rs"]
mod guest;
use core::ptr::addr_of_mut;
use guest::{hash, Arguments, Reader, MAX_VIEW};

// CompactSize needs one more byte than a u32 length above 65,535. A bounded
// view can contain at most sixteen scripts that large.
const SCRATCH_CAPACITY: usize = MAX_VIEW + 17;
static mut SCRATCH: [u8; SCRATCH_CAPACITY] = [0; SCRATCH_CAPACITY];

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

struct Writer<'a> {
    bytes: &'a mut [u8],
    offset: usize,
}

impl Writer<'_> {
    fn put(&mut self, bytes: &[u8]) -> Option<()> {
        let end = self.offset.checked_add(bytes.len())?;
        self.bytes.get_mut(self.offset..end)?.copy_from_slice(bytes);
        self.offset = end;
        Some(())
    }

    fn compact_size(&mut self, length: u32) -> Option<()> {
        if length < 253 {
            self.put(&[length as u8])
        } else if length <= u16::MAX as u32 {
            self.put(&[253])?;
            self.put(&(length as u16).to_le_bytes())
        } else {
            self.put(&[254])?;
            self.put(&length.to_le_bytes())
        }
    }

    fn digest(&self, output: &mut [u8]) -> Option<()> {
        hash(&self.bytes[..self.offset], output)
    }
}

fn native_witness_program(script: &[u8]) -> bool {
    matches!(script.first(), Some(0 | 0x51..=0x60))
        && matches!(script.get(1), Some(2..=40))
        && script.len() == script[1] as usize + 2
}

fn evaluate(arguments: Arguments) -> Option<bool> {
    let Arguments {
        program,
        parameters,
        view,
        witness,
    } = arguments;
    if !program.is_empty() || parameters.len() != 32 || !witness.is_empty() {
        return None;
    }
    let mut reader = Reader::new(view);
    let mut preimage = [0_u8; 84];
    preimage[..8].copy_from_slice(reader.take(8)?);
    let selected = reader.u32()?;
    let inputs = reader.u32()?;
    if selected >= inputs {
        return None;
    }
    preimage[8..12].copy_from_slice(&inputs.to_le_bytes());
    // The arena is disjoint from input arguments and the current stack. A
    // synchronous invocation exclusively owns this scratch space.
    let scratch = unsafe {
        core::slice::from_raw_parts_mut(addr_of_mut!(SCRATCH).cast::<u8>(), SCRATCH_CAPACITY)
    };
    let mut writer = Writer {
        bytes: scratch,
        offset: 0,
    };
    for _ in 0..inputs {
        reader.take(36)?; // Outpoints are not committed by CTV.
        writer.put(reader.take(4)?)?;
        reader.take(8)?; // Previous output amounts are not committed by CTV.
        let script_length = reader.u32()?;
        let script = reader.take(script_length as usize)?;
        // Native witness-program consensus requires an empty scriptSig. The
        // signed view cannot establish this for legacy or P2SH inputs.
        if !native_witness_program(script) {
            return Some(false);
        }
    }
    writer.digest(&mut preimage[12..44])?;
    writer.offset = 0;
    let outputs = reader.u32()?;
    preimage[44..48].copy_from_slice(&outputs.to_le_bytes());
    for _ in 0..outputs {
        writer.put(reader.take(8)?)?;
        let script_length = reader.u32()?;
        writer.compact_size(script_length)?;
        writer.put(reader.take(script_length as usize)?)?;
    }
    if !reader.is_finished() {
        return None;
    }
    writer.digest(&mut preimage[48..80])?;
    preimage[80..84].copy_from_slice(&selected.to_le_bytes());
    let mut digest = [0_u8; 32];
    hash(&preimage, &mut digest)?;
    Some(digest == parameters)
}
