//! Bounded argument memory and signed-view decoding shared by the examples.

use core::ptr::{addr_of, addr_of_mut};

pub const MAX_VIEW: usize = 1_048_576;
const MAX_ARGUMENT: usize = 65_536;
const INPUT_CAPACITY: usize = MAX_VIEW + 3 * MAX_ARGUMENT;

static mut INPUT: [u8; INPUT_CAPACITY] = [0; INPUT_CAPACITY];
static mut INPUT_USED: usize = 0;

#[link(wasm_import_module = "sapio_crypto_v1")]
extern "C" {
    fn sha256(pointer: u32, length: u32, output: u32) -> i32;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable()
}

/// Allocate disjoint input ranges in a fresh instance's fixed arena.
#[no_mangle]
pub extern "C" fn sapio_alloc_v1(length: u32) -> u32 {
    // Each invocation owns a fresh instance, with no reentrant guest imports.
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

pub fn evaluate(pointers: [u32; 8], evaluator: fn(Arguments<'_>) -> Option<bool>) -> i32 {
    let [pp, pl, ap, al, vp, vl, wp, wl] = pointers;
    if [pl, al, wl]
        .iter()
        .any(|length| *length as usize > MAX_ARGUMENT)
        || vl as usize > MAX_VIEW
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
        Some(true) => 1,
        Some(false) => 0,
        None => -1,
    }
}

fn input_slice(pointer: u32, length: u32) -> Option<&'static [u8]> {
    if length == 0 {
        return Some(&[]);
    }
    let pointer = pointer as usize;
    let length = length as usize;
    let start = addr_of!(INPUT).cast::<u8>() as usize;
    let offset = pointer.checked_sub(start)?;
    let end = offset.checked_add(length)?;
    // The guest only reads from its bounded, host-written allocation arena.
    unsafe {
        if end > INPUT_USED {
            return None;
        }
        Some(core::slice::from_raw_parts(pointer as *const u8, length))
    }
}

pub struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    pub fn take(&mut self, length: usize) -> Option<&'a [u8]> {
        let end = self.offset.checked_add(length)?;
        let bytes = self.bytes.get(self.offset..end)?;
        self.offset = end;
        Some(bytes)
    }

    pub fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    pub fn is_finished(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

#[allow(dead_code)]
pub fn hash(bytes: &[u8], output: &mut [u8]) -> Option<()> {
    if output.len() != 32 {
        return None;
    }
    // The runtime validates the memory ranges and writes one SHA256 digest.
    let result = unsafe {
        sha256(
            bytes.as_ptr() as u32,
            bytes.len() as u32,
            output.as_mut_ptr() as u32,
        )
    };
    (result == 0).then_some(())
}
