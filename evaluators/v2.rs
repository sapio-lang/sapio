//! Minimal WASM-v2 invocation glue; fragment logic lives in the SDK crate.

use core::ptr::{addr_of, addr_of_mut};
use sapio_covenant_fragments::{Failure, MAX_VIEW_BYTES};

const MAX_ARGUMENT: usize = 65_536;
const INPUT_CAPACITY: usize = MAX_VIEW_BYTES + 3 * MAX_ARGUMENT;
static mut INPUT: [u8; INPUT_CAPACITY] = [0; INPUT_CAPACITY];
static mut INPUT_USED: usize = 0;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable()
}

#[no_mangle]
pub extern "C" fn sapio_alloc_v2(length: u32) -> u32 {
    // The runtime creates a fresh instance per request and forbids reentrancy.
    unsafe {
        let used = INPUT_USED;
        let Some(end) = used.checked_add(length as usize) else {
            return 0;
        };
        if end > INPUT_CAPACITY {
            return 0;
        }
        INPUT_USED = end;
        addr_of_mut!(INPUT).cast::<u8>().add(used) as u32
    }
}

pub struct Arguments<'a> {
    pub program: &'a [u8],
    pub parameters: &'a [u8],
    pub view: &'a [u8],
    pub witness: &'a [u8],
}

pub fn evaluate(pointers: [u32; 8], evaluator: fn(Arguments<'_>) -> Result<bool, Failure>) -> i32 {
    let [pp, pl, ap, al, vp, vl, wp, wl] = pointers;
    if [pl, al, wl]
        .iter()
        .any(|length| *length as usize > MAX_ARGUMENT)
        || vl as usize > MAX_VIEW_BYTES
    {
        return -1;
    }
    let Some(program) = input_slice(pp, pl) else {
        return -1;
    };
    let Some(parameters) = input_slice(ap, al) else {
        return -1;
    };
    let Some(view) = input_slice(vp, vl) else {
        return -1;
    };
    let Some(witness) = input_slice(wp, wl) else {
        return -1;
    };
    match evaluator(Arguments {
        program,
        parameters,
        view,
        witness,
    }) {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(_) => -1,
    }
}

fn input_slice(pointer: u32, length: u32) -> Option<&'static [u8]> {
    if length == 0 {
        return Some(&[]);
    }
    let start = addr_of!(INPUT).cast::<u8>() as usize;
    let offset = (pointer as usize).checked_sub(start)?;
    let end = offset.checked_add(length as usize)?;
    // Nonempty inputs must lie wholly inside the host-written arena. The
    // fragment code never mutates it; its scratch storage is separate.
    unsafe {
        if end > INPUT_USED {
            return None;
        }
        Some(core::slice::from_raw_parts(
            pointer as *const u8,
            length as usize,
        ))
    }
}
