//! Allocation-free execution of bounded CAT and MID Tapscript profiles.
//!
//! Unsupported opcodes, including every OP_SUCCESS opcode, fail closed. This
//! deliberately bounded profile does not claim to be a consensus interpreter.
use crate::Failure;

const MAX_SCRIPT: usize = 10_000;
const MAX_WITNESS: usize = 65_536;
const MAX_ITEMS: usize = 1_000;
const MAX_ELEMENT: usize = 520;
const MAX_CONDITIONALS: usize = 100;

/// Authenticated transaction operations supplied by the evaluator boundary.
pub trait Crypto {
    /// Compute SHA256 over the exact supplied bytes.
    fn hash(&self, bytes: &[u8]) -> Result<[u8; 32], Failure>;
    /// Check a signature against the current transaction's selected leaf.
    fn check_signature(&self, signature: &[u8], key: &[u8]) -> Result<bool, Failure>;
    /// Apply BIP112 to the current input and this nonnegative Script number.
    fn check_sequence(&self, operand: i64) -> Result<bool, Failure>;
    /// Apply BIP65 to the selected input and this nonnegative Script number.
    fn check_lock_time(&self, _operand: i64) -> Result<bool, Failure> {
        Err(Failure::InvalidEncoding)
    }
    /// Verify a BIP348 signature over the exact supplied message.
    fn check_data_signature(
        &self,
        _signature: &[u8],
        _message: &[u8],
        _key: &[u8],
    ) -> Result<bool, Failure> {
        Err(Failure::InvalidEncoding)
    }
    /// Return x(lift_x(key) + tweak*G), rejecting invalid keys and scalars.
    fn tweak_add(&self, _tweak: &[u8; 32], _key: &[u8; 32]) -> Result<[u8; 32], Failure> {
        Err(Failure::InvalidEncoding)
    }
    /// Read the authenticated fields selected by the MID OP_TX profile.
    ///
    /// Selectors 0x8000, 0x10800 and 0x20800 return the current input's
    /// minimally encoded index, eight-byte LE amount and raw script. Selector
    /// 0x4300 takes an output index and returns its script and `Some(amount)`;
    /// every other selector must return `None` for the optional amount.
    fn transaction(
        &self,
        _selector: u32,
        _index: Option<u32>,
        _output: &mut [u8; 520],
    ) -> Result<(usize, Option<u64>), Failure> {
        Err(Failure::InvalidEncoding)
    }
}

#[derive(Clone, Copy)]
struct Item {
    bytes: [u8; MAX_ELEMENT],
    length: u16,
}

impl Item {
    const EMPTY: Self = Self {
        bytes: [0; MAX_ELEMENT],
        length: 0,
    };

    fn bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.length)]
    }
}

struct Machine {
    // Main stack grows from the front and altstack grows from the back. Their
    // shared capacity enforces BIP342's combined stack limit during transfers.
    items: [Item; MAX_ITEMS],
    main: usize,
    alternate: usize,
    signature_budget: usize,
}

impl Machine {
    const fn new() -> Self {
        Self {
            items: [Item::EMPTY; MAX_ITEMS],
            main: 0,
            alternate: 0,
            signature_budget: 0,
        }
    }

    fn need(&self, count: usize) -> Result<(), Failure> {
        if self.main < count {
            return Err(Failure::InvalidEncoding);
        }
        Ok(())
    }

    fn room(&self, count: usize) -> Result<(), Failure> {
        if self.main + self.alternate + count > MAX_ITEMS {
            return Err(Failure::InvalidEncoding);
        }
        Ok(())
    }

    fn push(&mut self, bytes: &[u8]) -> Result<(), Failure> {
        self.room(1)?;
        if bytes.len() > MAX_ELEMENT {
            return Err(Failure::InvalidEncoding);
        }
        let item = &mut self.items[self.main];
        item.bytes[..bytes.len()].copy_from_slice(bytes);
        item.length = bytes.len() as u16;
        self.main += 1;
        Ok(())
    }

    fn push_number(&mut self, number: i64) -> Result<(), Failure> {
        let mut bytes = [0; 9];
        let negative = number < 0;
        let mut magnitude = number.unsigned_abs();
        let mut length = 0;
        while magnitude != 0 {
            bytes[length] = magnitude as u8;
            magnitude >>= 8;
            length += 1;
        }
        if length != 0 {
            if bytes[length - 1] & 0x80 != 0 {
                bytes[length] = if negative { 0x80 } else { 0 };
                length += 1;
            } else if negative {
                bytes[length - 1] |= 0x80;
            }
        }
        self.push(&bytes[..length])
    }

    fn duplicate(&mut self, count: usize) -> Result<(), Failure> {
        self.need(count)?;
        self.room(count)?;
        for offset in 0..count {
            self.items[self.main + offset] = self.items[self.main - count + offset];
        }
        self.main += count;
        Ok(())
    }

    fn load_witness(&mut self, witness: &[u8], script_length: usize) -> Result<(), Failure> {
        self.main = 0;
        self.alternate = 0;
        if witness.len() > MAX_WITNESS {
            return Err(Failure::InvalidEncoding);
        }
        let mut cursor = 0;
        let count = read_length(witness, &mut cursor)?;
        if count > MAX_ITEMS {
            return Err(Failure::InvalidEncoding);
        }
        // BIP342 grants 50 units plus the consensus serialized witness size.
        // This virtual leaf has the supplied Script and a one-node control
        // block; its encoding is distinct from our u32-delimited guest ABI.
        let mut witness_size =
            compact_length(count + 2) + compact_length(script_length) + script_length + 1 + 33;
        for _ in 0..count {
            let length = read_length(witness, &mut cursor)?;
            let bytes = take(witness, &mut cursor, length)?;
            self.push(bytes)?;
            witness_size += compact_length(length) + length;
        }
        if cursor != witness.len() {
            return Err(Failure::InvalidEncoding);
        }
        self.signature_budget = 50 + witness_size;
        Ok(())
    }

    fn run(
        &mut self,
        script: &[u8],
        witness: &[u8],
        crypto: &impl Crypto,
        mid: bool,
    ) -> Result<bool, Failure> {
        if script.len() > MAX_SCRIPT {
            return Err(Failure::InvalidEncoding);
        }
        self.load_witness(witness, script.len())?;
        let mut cursor = 0;
        let mut conditions = [false; MAX_CONDITIONALS];
        let mut depth = 0;
        let mut inactive = 0;
        while cursor < script.len() {
            let opcode = take(script, &mut cursor, 1)?[0];
            if mid && matches!(opcode, 0x63 | 0x64 | 0x67 | 0x68) {
                match opcode {
                    0x63 | 0x64 => {
                        if depth == MAX_CONDITIONALS {
                            return Err(Failure::InvalidEncoding);
                        }
                        let mut selected = false;
                        if inactive == 0 {
                            self.need(1)?;
                            self.main -= 1;
                            selected = match self.items[self.main].bytes() {
                                [] => false,
                                [1] => true,
                                _ => return Err(Failure::InvalidEncoding),
                            };
                            if opcode == 0x64 {
                                selected = !selected;
                            }
                        }
                        conditions[depth] = selected;
                        depth += 1;
                        inactive += usize::from(!selected);
                    }
                    0x67 => {
                        if depth == 0 {
                            return Err(Failure::InvalidEncoding);
                        }
                        let current = &mut conditions[depth - 1];
                        if *current {
                            inactive += 1;
                        } else {
                            inactive -= 1;
                        }
                        *current = !*current;
                    }
                    _ => {
                        if depth == 0 {
                            return Err(Failure::InvalidEncoding);
                        }
                        depth -= 1;
                        inactive -= usize::from(!conditions[depth]);
                    }
                }
                continue;
            }
            if mid && inactive != 0 && !(0x01..=0x4e).contains(&opcode) {
                if !supported_mid(opcode) {
                    return Err(Failure::InvalidEncoding);
                }
                continue;
            }
            match opcode {
                0x00 => self.push(&[])?,
                0x01..=0x4e => {
                    let length = match opcode {
                        0x01..=0x4b => usize::from(opcode),
                        0x4c => usize::from(take(script, &mut cursor, 1)?[0]),
                        0x4d => {
                            let bytes = take(script, &mut cursor, 2)?;
                            usize::from(u16::from_le_bytes([bytes[0], bytes[1]]))
                        }
                        // Every PUSHDATA4 would be nonminimal under our
                        // 520-byte element limit; do not allocate its length.
                        _ => return Err(Failure::InvalidEncoding),
                    };
                    let bytes = take(script, &mut cursor, length)?;
                    if !minimal_push(opcode, bytes) || bytes.len() > MAX_ELEMENT {
                        return Err(Failure::InvalidEncoding);
                    }
                    if inactive == 0 {
                        self.push(bytes)?;
                    }
                }
                0x4f => self.push_number(-1)?,
                0x51..=0x60 => self.push_number(i64::from(opcode - 0x50))?,
                0x69 => {
                    self.need(1)?;
                    self.main -= 1;
                    if !truth(self.items[self.main].bytes()) {
                        return Ok(false);
                    }
                }
                0x6b => {
                    self.need(1)?;
                    self.main -= 1;
                    self.alternate += 1;
                    self.items[MAX_ITEMS - self.alternate] = self.items[self.main];
                }
                0x6c => {
                    if self.alternate == 0 {
                        return Err(Failure::InvalidEncoding);
                    }
                    self.items[self.main] = self.items[MAX_ITEMS - self.alternate];
                    self.alternate -= 1;
                    self.main += 1;
                }
                0x6e => self.duplicate(2)?,
                0x6d if mid => {
                    self.need(2)?;
                    self.main -= 2;
                }
                0x72 if mid => {
                    self.need(4)?;
                    self.items.swap(self.main - 4, self.main - 2);
                    self.items.swap(self.main - 3, self.main - 1);
                }
                0x75 => {
                    self.need(1)?;
                    self.main -= 1;
                }
                0x76 => self.duplicate(1)?,
                0x77 if mid => {
                    self.need(2)?;
                    self.items[self.main - 2] = self.items[self.main - 1];
                    self.main -= 1;
                }
                0x78 if mid => {
                    self.need(2)?;
                    let item = self.items[self.main - 2];
                    self.push(item.bytes())?;
                }
                0x79 | 0x7a if mid => {
                    self.need(1)?;
                    self.main -= 1;
                    let index = script_number(self.items[self.main].bytes(), 4)?;
                    let index = usize::try_from(index).map_err(|_| Failure::InvalidEncoding)?;
                    self.need(index.checked_add(1).ok_or(Failure::InvalidEncoding)?)?;
                    let position = self.main - index - 1;
                    let item = self.items[position];
                    if opcode == 0x7a {
                        self.items.copy_within(position + 1..self.main, position);
                        self.main -= 1;
                    }
                    self.push(item.bytes())?;
                }
                0x7b => {
                    self.need(3)?;
                    self.items.swap(self.main - 3, self.main - 2);
                    self.items.swap(self.main - 2, self.main - 1);
                }
                0x7c => {
                    self.need(2)?;
                    self.items.swap(self.main - 2, self.main - 1);
                }
                0x7d if mid => {
                    self.need(2)?;
                    self.room(1)?;
                    let top = self.items[self.main - 1];
                    self.items[self.main] = top;
                    self.items[self.main - 1] = self.items[self.main - 2];
                    self.items[self.main - 2] = top;
                    self.main += 1;
                }
                0x7e => {
                    self.need(2)?;
                    let (prefix, tail) = self.items.split_at_mut(self.main - 1);
                    let left = &mut prefix[self.main - 2];
                    let right = &tail[0];
                    let left_length = usize::from(left.length);
                    let length = left_length + usize::from(right.length);
                    if length > MAX_ELEMENT {
                        return Err(Failure::InvalidEncoding);
                    }
                    left.bytes[left_length..length].copy_from_slice(right.bytes());
                    left.length = length as u16;
                    self.main -= 1;
                }
                0x82 => {
                    self.need(1)?;
                    self.push_number(i64::from(self.items[self.main - 1].length))?;
                }
                0x87 | 0x88 => {
                    self.need(2)?;
                    let equal =
                        self.items[self.main - 2].bytes() == self.items[self.main - 1].bytes();
                    self.main -= 2;
                    if opcode == 0x88 {
                        if !equal {
                            return Ok(false);
                        }
                    } else {
                        self.push_number(i64::from(equal))?;
                    }
                }
                0x8b | 0x90..=0x92 if mid => {
                    self.need(1)?;
                    self.main -= 1;
                    let operand = script_number(self.items[self.main].bytes(), 4)?;
                    self.push_number(match opcode {
                        0x8b => operand + 1,
                        0x90 => operand.abs(),
                        0x91 => i64::from(operand == 0),
                        _ => i64::from(operand != 0),
                    })?;
                }
                0x93 | 0x94 | 0x9a | 0x9d | 0xa1 | 0xa2 if mid => {
                    self.need(2)?;
                    let left = script_number(self.items[self.main - 2].bytes(), 4)?;
                    let right = script_number(self.items[self.main - 1].bytes(), 4)?;
                    self.main -= 2;
                    let result = match opcode {
                        0x93 => left + right,
                        0x94 => left - right,
                        0x9a => i64::from(left != 0 && right != 0),
                        0x9d => {
                            if left != right {
                                return Ok(false);
                            }
                            continue;
                        }
                        0xa1 => i64::from(left <= right),
                        _ => i64::from(left >= right),
                    };
                    self.push_number(result)?;
                }
                0x9f | 0xa0 => {
                    self.need(2)?;
                    let left = script_number(self.items[self.main - 2].bytes(), 4)?;
                    let right = script_number(self.items[self.main - 1].bytes(), 4)?;
                    let result = if opcode == 0x9f {
                        left < right
                    } else {
                        left > right
                    };
                    self.main -= 2;
                    self.push_number(i64::from(result))?;
                }
                0xa8 | 0xaa => {
                    self.need(1)?;
                    let mut digest = crypto.hash(self.items[self.main - 1].bytes())?;
                    if opcode == 0xaa {
                        digest = crypto.hash(&digest)?;
                    }
                    self.main -= 1;
                    self.push(&digest)?;
                }
                0xac | 0xad => {
                    self.need(2)?;
                    let signature = self.items[self.main - 2].bytes();
                    let key = self.items[self.main - 1].bytes();
                    if key.len() != 32 {
                        return Err(Failure::Signature);
                    }
                    let valid = if signature.is_empty() {
                        false
                    } else {
                        if signature.len() != 64 && !(signature.len() == 65 && signature[64] == 1) {
                            return Err(Failure::Signature);
                        }
                        self.signature_budget = self
                            .signature_budget
                            .checked_sub(50)
                            .ok_or(Failure::Signature)?;
                        if !crypto.check_signature(signature, key)? {
                            return Err(Failure::Signature);
                        }
                        true
                    };
                    self.main -= 2;
                    if opcode == 0xad {
                        if !valid {
                            return Ok(false);
                        }
                    } else {
                        self.push_number(i64::from(valid))?;
                    }
                }
                0xb2 => {
                    self.need(1)?;
                    let sequence = script_number(self.items[self.main - 1].bytes(), 5)?;
                    if sequence < 0 || !crypto.check_sequence(sequence)? {
                        return Ok(false);
                    }
                }
                0xb1 if mid => {
                    self.need(1)?;
                    let lock_time = script_number(self.items[self.main - 1].bytes(), 5)?;
                    if lock_time < 0 || !crypto.check_lock_time(lock_time)? {
                        return Ok(false);
                    }
                }
                0xbf if mid => {
                    self.need(1)?;
                    self.main -= 1;
                    let selector = script_number(self.items[self.main].bytes(), 4)?;
                    let selector = u32::try_from(selector).map_err(|_| Failure::InvalidEncoding)?;
                    let index = match selector {
                        0x8000 | 0x10800 | 0x20800 => None,
                        0x4300 => {
                            self.need(1)?;
                            self.main -= 1;
                            let index = script_number(self.items[self.main].bytes(), 4)?;
                            Some(u32::try_from(index).map_err(|_| Failure::InvalidEncoding)?)
                        }
                        _ => return Err(Failure::InvalidEncoding),
                    };
                    let mut output = [0; MAX_ELEMENT];
                    let (length, amount) = crypto.transaction(selector, index, &mut output)?;
                    if length > MAX_ELEMENT || amount.is_some() != (selector == 0x4300) {
                        return Err(Failure::InvalidEncoding);
                    }
                    if selector == 0x10800 && length != 8 {
                        return Err(Failure::InvalidEncoding);
                    }
                    if selector == 0x8000 && script_number(&output[..length], 4)? < 0 {
                        return Err(Failure::InvalidEncoding);
                    }
                    if let Some(amount) = amount {
                        self.push(&amount.to_le_bytes())?;
                    }
                    self.push(&output[..length])?;
                }
                0xc0 if mid => {
                    self.need(2)?;
                    let tweak = self.items[self.main - 2]
                        .bytes()
                        .try_into()
                        .map_err(|_| Failure::InvalidEncoding)?;
                    let key = self.items[self.main - 1]
                        .bytes()
                        .try_into()
                        .map_err(|_| Failure::InvalidEncoding)?;
                    self.signature_budget = self
                        .signature_budget
                        .checked_sub(50)
                        .ok_or(Failure::Signature)?;
                    let output = crypto.tweak_add(tweak, key)?;
                    self.main -= 2;
                    self.push(&output)?;
                }
                0xcc if mid => {
                    self.need(3)?;
                    let signature = self.items[self.main - 3].bytes();
                    let message = self.items[self.main - 2].bytes();
                    let key = self.items[self.main - 1].bytes();
                    if key.len() != 32 || (!signature.is_empty() && signature.len() != 64) {
                        return Err(Failure::Signature);
                    }
                    let valid = if signature.is_empty() {
                        false
                    } else {
                        self.signature_budget = self
                            .signature_budget
                            .checked_sub(50)
                            .ok_or(Failure::Signature)?;
                        if !crypto.check_data_signature(signature, message, key)? {
                            return Err(Failure::Signature);
                        }
                        true
                    };
                    self.main -= 3;
                    self.push_number(i64::from(valid))?;
                }
                _ => return Err(Failure::InvalidEncoding),
            }
        }
        // Tapscript requires exactly one true main-stack result. The altstack
        // is not part of cleanstack and may retain values, as in Bitcoin Core.
        Ok(depth == 0 && self.main == 1 && truth(self.items[0].bytes()))
    }
}

fn take<'a>(bytes: &'a [u8], cursor: &mut usize, length: usize) -> Result<&'a [u8], Failure> {
    let end = cursor.checked_add(length).ok_or(Failure::InvalidEncoding)?;
    let result = bytes.get(*cursor..end).ok_or(Failure::InvalidEncoding)?;
    *cursor = end;
    Ok(result)
}

fn read_length(bytes: &[u8], cursor: &mut usize) -> Result<usize, Failure> {
    let encoded = take(bytes, cursor, 4)?;
    Ok(u32::from_le_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]) as usize)
}

fn compact_length(value: usize) -> usize {
    match value {
        0..=252 => 1,
        253..=65_535 => 3,
        _ => 5,
    }
}

fn minimal_push(opcode: u8, bytes: &[u8]) -> bool {
    match bytes {
        [] => opcode == 0,
        [1..=16] | [0x81] => false,
        _ if bytes.len() <= 75 => usize::from(opcode) == bytes.len(),
        _ if bytes.len() <= 255 => opcode == 0x4c,
        _ => opcode == 0x4d,
    }
}

fn truth(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .enumerate()
        .any(|(index, byte)| *byte != 0 && !(index + 1 == bytes.len() && *byte == 0x80))
}

fn script_number(bytes: &[u8], maximum_length: usize) -> Result<i64, Failure> {
    if bytes.len() > maximum_length {
        return Err(Failure::InvalidEncoding);
    }
    let Some(last) = bytes.last() else {
        return Ok(0);
    };
    if last & 0x7f == 0 && (bytes.len() == 1 || bytes[bytes.len() - 2] & 0x80 == 0) {
        return Err(Failure::InvalidEncoding);
    }
    let mut value = 0_i64;
    for (index, byte) in bytes.iter().enumerate() {
        value |= i64::from(*byte) << (8 * index);
    }
    if last & 0x80 != 0 {
        value &= !(0x80_i64 << (8 * (bytes.len() - 1)));
        value = -value;
    }
    Ok(value)
}

fn supported_mid(opcode: u8) -> bool {
    matches!(
        opcode,
        0x00..=0x4f
            | 0x51..=0x60
            | 0x69
            | 0x6b..=0x6e
            | 0x72
            | 0x75..=0x7e
            | 0x82
            | 0x87..=0x88
            | 0x8b
            | 0x90..=0x94
            | 0x9a
            | 0x9d
            | 0x9f..=0xa2
            | 0xa8
            | 0xaa
            | 0xac..=0xad
            | 0xb1..=0xb2
            | 0xbf..=0xc0
            | 0xcc
    )
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
static mut MACHINE: Machine = Machine::new();

// Native callers may run concurrently. The guest has a fresh instance per
// invocation and forbids reentrancy; its fixed arena needs neither a lock nor
// an allocator. Static initialization avoids a half-megabyte stack temporary.
#[cfg(not(target_arch = "wasm32"))]
static MACHINE: std::sync::Mutex<Machine> = std::sync::Mutex::new(Machine::new());

/// Execute Script with a u32-count/u32-length encoded initial stack.
///
/// Resource limits are 10,000 Script bytes, 65,536 evidence bytes, 1,000 combined
/// main/alternate stack elements, and 520 bytes per element including CAT's
/// result. Pushes and CSV numbers must use minimal encodings. CHECKSIG accepts
/// only 32-byte keys and DEFAULT/ALL signatures and enforces BIP342 NULLFAIL.
pub fn evaluate(script: &[u8], witness: &[u8], crypto: &impl Crypto) -> Result<bool, Failure> {
    execute_profile(script, witness, crypto, false)
}

/// Execute the MID profile with CAT, OP_TX, CSFS and OP_TWEAKADD.
///
/// This retains the CAT profile's bounds and strict encodings, while adding
/// current four-byte Script arithmetic, stack manipulation, BIP65 and bounded
/// MINIMALIF conditionals. The candidate opcode numbers are the LayerTree
/// reference simulator's placeholders: TX=0xbf, TWEAKADD=0xc0 and CSFS=0xcc.
/// Only OP_TX selectors 0x8000, 0x10800, 0x20800 and 0x4300 are supported.
pub fn evaluate_mid(script: &[u8], witness: &[u8], crypto: &impl Crypto) -> Result<bool, Failure> {
    execute_profile(script, witness, crypto, true)
}

fn execute_profile(
    script: &[u8],
    witness: &[u8],
    crypto: &impl Crypto,
    mid: bool,
) -> Result<bool, Failure> {
    #[cfg(target_arch = "wasm32")]
    let machine = unsafe { &mut *core::ptr::addr_of_mut!(MACHINE) };
    #[cfg(not(target_arch = "wasm32"))]
    let mut guard = MACHINE.lock().expect("Script execution arena poisoned");
    #[cfg(not(target_arch = "wasm32"))]
    let machine = &mut *guard;
    machine.run(script, witness, crypto, mid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, vec, vec::Vec};

    #[derive(Default)]
    struct TestCrypto {
        hashes: RefCell<Vec<Vec<u8>>>,
        sequences: RefCell<Vec<i64>>,
        signatures: RefCell<Vec<(Vec<u8>, Vec<u8>)>>,
    }

    impl Crypto for TestCrypto {
        fn hash(&self, bytes: &[u8]) -> Result<[u8; 32], Failure> {
            self.hashes.borrow_mut().push(bytes.to_vec());
            Ok([bytes.len() as u8; 32])
        }

        fn check_signature(&self, signature: &[u8], key: &[u8]) -> Result<bool, Failure> {
            self.signatures
                .borrow_mut()
                .push((signature.to_vec(), key.to_vec()));
            Ok(signature[0] != 0)
        }

        fn check_sequence(&self, operand: i64) -> Result<bool, Failure> {
            self.sequences.borrow_mut().push(operand);
            Ok(operand == 20 || operand == 0x8000_0000)
        }
    }

    fn evidence(items: &[&[u8]]) -> Vec<u8> {
        let mut encoded = Vec::from((items.len() as u32).to_le_bytes());
        for item in items {
            encoded.extend_from_slice(&(item.len() as u32).to_le_bytes());
            encoded.extend_from_slice(item);
        }
        encoded
    }

    fn execute(script: &[u8], items: &[&[u8]]) -> Result<bool, Failure> {
        evaluate(script, &evidence(items), &TestCrypto::default())
    }

    #[derive(Default)]
    struct MidCrypto {
        basic: TestCrypto,
        calls: RefCell<Vec<(u32, Option<u32>)>>,
    }

    impl Crypto for MidCrypto {
        fn hash(&self, bytes: &[u8]) -> Result<[u8; 32], Failure> {
            self.basic.hash(bytes)
        }

        fn check_signature(&self, signature: &[u8], key: &[u8]) -> Result<bool, Failure> {
            self.basic.check_signature(signature, key)
        }

        fn check_sequence(&self, operand: i64) -> Result<bool, Failure> {
            self.basic.check_sequence(operand)
        }

        fn check_lock_time(&self, operand: i64) -> Result<bool, Failure> {
            Ok(operand == 500 || operand == 0x8000_0000)
        }

        fn check_data_signature(
            &self,
            signature: &[u8],
            message: &[u8],
            key: &[u8],
        ) -> Result<bool, Failure> {
            Ok(signature[0] == 1 && message == b"data" && key == [4; 32])
        }

        fn tweak_add(&self, tweak: &[u8; 32], key: &[u8; 32]) -> Result<[u8; 32], Failure> {
            if tweak != &[3; 32] || key != &[4; 32] {
                return Err(Failure::TweakProof);
            }
            Ok([5; 32])
        }

        fn transaction(
            &self,
            selector: u32,
            index: Option<u32>,
            output: &mut [u8; 520],
        ) -> Result<(usize, Option<u64>), Failure> {
            self.calls.borrow_mut().push((selector, index));
            match (selector, index) {
                (0x8000, None) => Ok((0, None)),
                (0x10800, None) => {
                    output[..8].copy_from_slice(&123_u64.to_le_bytes());
                    Ok((8, None))
                }
                (0x20800, None) => {
                    output[..2].copy_from_slice(&[0x51, 0x21]);
                    Ok((2, None))
                }
                (0x4300, Some(1)) => {
                    output[..2].copy_from_slice(&[0x51, 0x21]);
                    Ok((2, Some(999)))
                }
                _ => Err(Failure::InvalidEncoding),
            }
        }
    }

    fn execute_mid(script: &[u8], items: &[&[u8]]) -> Result<bool, Failure> {
        evaluate_mid(script, &evidence(items), &MidCrypto::default())
    }

    #[test]
    fn mid_control_flow_requires_minimal_if_and_balanced_bounded_nesting() {
        assert_eq!(
            execute_mid(&[0x63, 0x51, 0x67, 0x00, 0x68], &[&[1]]),
            Ok(true)
        );
        assert_eq!(
            execute_mid(&[0x64, 0x00, 0x67, 0x51, 0x68], &[&[1]]),
            Ok(true)
        );
        // Inner branches under an inactive outer branch do not consume items.
        assert_eq!(
            execute_mid(
                &[0x00, 0x63, 0x63, 0x75, 0x67, 0x75, 0x68, 0x67, 0x51, 0x68],
                &[]
            ),
            Ok(true)
        );
        for condition in [&[2][..], &[0][..], &[0x80][..], &[1, 0][..]] {
            assert!(execute_mid(&[0x63, 0x51, 0x68], &[condition]).is_err());
        }
        for malformed in [&[0x67][..], &[0x68][..], &[0x63][..]] {
            assert!(execute_mid(malformed, &[]).is_err());
        }
        assert_eq!(execute_mid(&[0x51, 0x63, 0x51], &[]), Ok(false));
        let nested = |depth| {
            let mut script = [0x51, 0x63].repeat(depth);
            script.push(0x51);
            script.extend(vec![0x68; depth]);
            script
        };
        assert_eq!(execute_mid(&nested(MAX_CONDITIONALS), &[]), Ok(true));
        assert!(execute_mid(&nested(MAX_CONDITIONALS + 1), &[]).is_err());
        assert!(execute_mid(&[0x00, 0x63, 0xbb, 0x68, 0x51], &[]).is_err());
        assert!(execute_mid(&[0x00, 0x63, 3, 1, 0x68], &[]).is_err());
    }

    #[test]
    fn mid_stack_operations_preserve_order_and_reject_invalid_indices() {
        assert_eq!(
            execute_mid(
                &[0x72, 0x52, 0x88, 0x51, 0x88, 0x54, 0x88, 0x53, 0x87],
                &[&[1], &[2], &[3], &[4]]
            ),
            Ok(true)
        );
        assert_eq!(
            execute_mid(&[0x78, 0x51, 0x88, 0x52, 0x88, 0x51, 0x87], &[&[1], &[2]]),
            Ok(true)
        );
        assert_eq!(
            execute_mid(&[0x7d, 0x52, 0x88, 0x51, 0x88, 0x52, 0x87], &[&[1], &[2]]),
            Ok(true)
        );
        assert_eq!(execute_mid(&[0x77, 0x52, 0x87], &[&[1], &[2]]), Ok(true));
        assert_eq!(execute_mid(&[0x6d, 0x51], &[&[1], &[2]]), Ok(true));
        assert_eq!(
            execute_mid(
                &[0x52, 0x79, 0x51, 0x88, 0x6d, 0x51, 0x87],
                &[&[1], &[2], &[3]]
            ),
            Ok(true)
        );
        assert_eq!(
            execute_mid(
                &[0x52, 0x7a, 0x51, 0x88, 0x53, 0x88, 0x52, 0x87],
                &[&[1], &[2], &[3]]
            ),
            Ok(true)
        );
        for index in [&[0x81][..], &[3][..], &[0][..], &[0, 0, 0, 0x80, 0][..]] {
            assert!(execute_mid(&[0x79], &[&[1], index]).is_err());
            assert!(execute_mid(&[0x7a], &[&[1], index]).is_err());
        }
    }

    #[test]
    fn mid_arithmetic_uses_minimal_four_byte_operands_and_five_byte_results() {
        assert_eq!(execute_mid(&[0x93, 0x55, 0x87], &[&[2], &[3]]), Ok(true));
        assert_eq!(execute_mid(&[0x94, 0x4f, 0x87], &[&[2], &[3]]), Ok(true));
        assert_eq!(execute_mid(&[0x90, 0x51, 0x87], &[&[0x81]]), Ok(true));
        assert_eq!(execute_mid(&[0x8b, 0x52, 0x87], &[&[1]]), Ok(true));
        assert_eq!(execute_mid(&[0x91], &[&[]]), Ok(true));
        assert_eq!(execute_mid(&[0x92], &[&[0x81]]), Ok(true));
        assert_eq!(execute_mid(&[0x9a], &[&[0x81], &[2]]), Ok(true));
        assert_eq!(execute_mid(&[0xa1], &[&[2], &[3]]), Ok(true));
        assert_eq!(execute_mid(&[0xa2], &[&[3], &[2]]), Ok(true));
        assert_eq!(execute_mid(&[0x9d, 0x51], &[&[2], &[2]]), Ok(true));
        assert_eq!(execute_mid(&[0x9d, 0x51], &[&[2], &[3]]), Ok(false));
        let maximum = [0xff, 0xff, 0xff, 0x7f];
        assert_eq!(
            execute_mid(&[0x8b, 5, 0, 0, 0, 0x80, 0, 0x87], &[&maximum]),
            Ok(true)
        );
        assert!(execute_mid(&[0x8b, 0x8b], &[&maximum]).is_err());
        for nonminimal in [&[0][..], &[0x80][..], &[1, 0][..]] {
            assert!(execute_mid(&[0x90], &[nonminimal]).is_err());
            assert!(execute_mid(&[0x93], &[nonminimal, &[1]]).is_err());
        }
        // MID's NONNEG check rejects a negative amount before it is hashed.
        assert_eq!(
            execute_mid(&[0x76, 0x76, 0x90, 0x88, 0x75, 0x51], &[&[0x81]]),
            Ok(false)
        );
    }

    #[test]
    fn mid_transaction_selectors_use_raw_amounts_and_ordered_output_fields() {
        let crypto = MidCrypto::default();
        let script = [
            3, 0, 0x80, 0, 0xbf, 0x00, 0x88, 3, 0, 8, 1, 0xbf, 8, 123, 0, 0, 0, 0, 0, 0, 0, 0x88,
            3, 0, 8, 2, 0xbf, 2, 0x51, 0x21, 0x88, 0x51, 2, 0, 0x43, 0xbf, 2, 0x51, 0x21, 0x88, 8,
            0xe7, 3, 0, 0, 0, 0, 0, 0, 0x87,
        ];
        assert_eq!(evaluate_mid(&script, &evidence(&[]), &crypto), Ok(true));
        assert_eq!(
            &*crypto.calls.borrow(),
            &[
                (0x8000, None),
                (0x10800, None),
                (0x20800, None),
                (0x4300, Some(1))
            ]
        );
        assert!(execute_mid(&[0x51, 0xbf], &[]).is_err());
        assert!(execute_mid(&[0x4f, 2, 0, 0x43, 0xbf], &[]).is_err());
        assert!(execute_mid(&[0x00, 2, 0, 0x43, 0xbf], &[]).is_err());
        assert!(execute_mid(&[3, 0, 8, 0x81, 0xbf], &[]).is_err());
    }

    #[test]
    fn mid_crypto_hooks_are_strict_and_old_profile_remains_closed() {
        assert_eq!(
            execute_mid(&[0xcc], &[&[1; 64], b"data", &[4; 32]]),
            Ok(true)
        );
        assert_eq!(execute_mid(&[0xcc], &[&[], b"data", &[4; 32]]), Ok(false));
        assert_eq!(
            execute_mid(&[0xcc], &[&[0; 64], b"data", &[4; 32]]),
            Err(Failure::Signature)
        );
        assert_eq!(
            execute_mid(&[0xcc], &[&[1; 65], b"data", &[4; 32]]),
            Err(Failure::Signature)
        );
        assert_eq!(
            execute_mid(
                &[0xc0, 0x82, 1, 32, 0x88, 0x75, 0x51],
                &[&[3; 32], &[4; 32]]
            ),
            Ok(true)
        );
        assert!(execute_mid(&[0xc0], &[&[3; 31], &[4; 32]]).is_err());
        assert!(execute_mid(&[0xc0], &[&[2; 32], &[4; 32]]).is_err());
        assert_eq!(execute_mid(&[0xb1, 0x75, 0x51], &[&[0xf4, 1]]), Ok(true));
        assert_eq!(
            execute_mid(&[0xb1, 0x75, 0x51], &[&[0, 0, 0, 0x80, 0]]),
            Ok(true)
        );
        assert_eq!(execute_mid(&[0xb1], &[&[0x81]]), Ok(false));
        for opcode in [
            0x63, 0x6d, 0x72, 0x77, 0x78, 0x79, 0x7a, 0x7d, 0x8b, 0x90, 0x91, 0x92, 0x93, 0x94,
            0x9a, 0x9d, 0xa1, 0xa2, 0xb1, 0xbf, 0xc0, 0xcc,
        ] {
            assert!(execute(&[opcode], &[&[1], &[2], &[3]]).is_err());
        }
        assert!(evaluate_mid(
            &[0xc0],
            &evidence(&[&[3; 32], &[4; 32]]),
            &TestCrypto::default()
        )
        .is_err());
    }

    #[test]
    fn cat_preserves_stack_order_and_520_byte_limit() {
        assert_eq!(
            execute(&[0x7e, 3, b'a', b'b', b'c', 0x87], &[b"a", b"bc"]),
            Ok(true)
        );
        let left = [1; 260];
        let right = [2; 260];
        assert_eq!(
            execute(&[0x7e, 0x82, 2, 8, 2, 0x88, 0x75, 0x51], &[&left, &right]),
            Ok(true)
        );
        assert!(execute(&[0x7e], &[&left, &[2; 261]]).is_err());
        assert!(execute(&[0x7e], &[b"a"]).is_err());
        assert!(execute(&[0x51], &[&[0; 521]]).is_err());
    }

    #[test]
    fn stack_operations_follow_bitcoin_order() {
        assert_eq!(
            execute(
                &[0x7b, 0x52, 0x88, 0x54, 0x88, 0x53, 0x87],
                &[&[2], &[3], &[4]]
            ),
            Ok(true)
        );
        assert_eq!(
            execute(&[0x7c, 0x52, 0x88, 0x53, 0x87], &[&[2], &[3]]),
            Ok(true)
        );
        assert_eq!(
            execute(&[0x6e, 0x7e, 0x6b, 0x7e, 0x6c, 0x87], &[b"a", b"b"]),
            Ok(true)
        );
        assert_eq!(execute(&[0x76, 0x88, 0x51], &[b"a"]), Ok(true));
        assert!(execute(&[0x6c], &[]).is_err());
        assert!(execute(&[0x6e], &[b"a"]).is_err());
    }

    #[test]
    fn cleanstack_and_negative_zero_use_script_truth() {
        for false_item in [&[][..], &[0][..], &[0x80][..], &[0, 0x80][..]] {
            assert_eq!(execute(&[], &[false_item]), Ok(false));
        }
        assert_eq!(execute(&[], &[&[0x80, 0]]), Ok(true));
        assert_eq!(execute(&[], &[&[0x81]]), Ok(true));
        assert_eq!(execute(&[0x51, 0x51], &[]), Ok(false));
        assert_eq!(execute(&[0x51, 0x6b, 0x51], &[]), Ok(true));
        assert_eq!(execute(&[0x69, 0x51], &[&[0x80]]), Ok(false));
    }

    #[test]
    fn shared_stack_limit_holds_during_altstack_transfers() {
        let items = vec![&[][..]; MAX_ITEMS];
        assert!(execute(&[0x51], &items).is_err());
        assert!(execute(&[0x6b, 0x51], &items).is_err());
        let mut script = vec![0x6b; MAX_ITEMS];
        script.extend([0x6c, 0x75].repeat(MAX_ITEMS));
        script.push(0x51);
        assert_eq!(execute(&script, &items), Ok(true));
        assert!(execute(&[], &vec![&[][..]; MAX_ITEMS + 1]).is_err());
    }

    #[test]
    fn unsupported_and_malformed_script_never_succeed() {
        for opcode in [0x50, 0x61, 0x63, 0x7f, 0x80, 0x89, 0xbb, 0xff] {
            assert!(execute(&[opcode], &[&[1]]).is_err());
        }
        for script in [
            &[2, 1][..],
            &[0x4c][..],
            &[0x4d, 1][..],
            &[0x4e, 0, 0, 0, 0][..],
        ] {
            assert!(execute(script, &[]).is_err());
        }
        for script in [
            &[1, 1][..],
            &[1, 0x81][..],
            &[0x4c, 1, 42][..],
            &[0x4c, 0][..],
        ] {
            assert!(execute(script, &[]).is_err());
        }
        assert!(execute(&vec![0x51; MAX_SCRIPT + 1], &[]).is_err());
    }

    #[test]
    fn witness_is_bounded_and_fully_consumed() {
        for bytes in [
            &[][..],
            &[1, 0, 0][..],
            &[1, 0, 0, 0][..],
            &[0, 0, 0, 0, 0][..],
        ] {
            assert!(evaluate(&[0x51], bytes, &TestCrypto::default()).is_err());
        }
        let mut encoded = evidence(&[&[1]]);
        encoded[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(evaluate(&[], &encoded, &TestCrypto::default()).is_err());
        assert!(evaluate(&[], &vec![0; MAX_WITNESS + 1], &TestCrypto::default()).is_err());
    }

    #[test]
    fn hash_opcodes_pass_exact_bytes_and_hash256_runs_twice() {
        let crypto = TestCrypto::default();
        assert_eq!(evaluate(&[0xaa], &evidence(&[b"abc"]), &crypto), Ok(true));
        assert_eq!(&*crypto.hashes.borrow(), &[b"abc".to_vec(), vec![3; 32]]);
        let crypto = TestCrypto::default();
        assert_eq!(evaluate(&[0xa8], &evidence(&[b"abc"]), &crypto), Ok(true));
        assert_eq!(&*crypto.hashes.borrow(), &[b"abc".to_vec()]);
    }

    #[test]
    fn comparisons_consume_ordered_minimal_four_byte_numbers() {
        assert_eq!(execute(&[0x9f], &[&[1], &[2]]), Ok(true));
        assert_eq!(execute(&[0xa0], &[&[2], &[1]]), Ok(true));
        assert_eq!(execute(&[0x9f], &[&[2], &[1]]), Ok(false));
        assert_eq!(execute(&[0xa0], &[&[1], &[2]]), Ok(false));
        assert_eq!(execute(&[0x9f], &[&[1], &[1]]), Ok(false));
        assert_eq!(execute(&[0xa0], &[&[], &[]]), Ok(false));
        assert_eq!(execute(&[0x9f], &[&[0x81], &[]]), Ok(true));
        assert_eq!(
            execute(
                &[0x9f],
                &[&[0xff, 0xff, 0xff, 0xff], &[0xff, 0xff, 0xff, 0x7f]]
            ),
            Ok(true)
        );
        // Comparison results are exactly Script's canonical bool encodings.
        assert_eq!(execute(&[0x9f, 0x51, 0x87], &[&[1], &[2]]), Ok(true));
        assert_eq!(execute(&[0x9f, 0x00, 0x87], &[&[2], &[1]]), Ok(true));
        for nonminimal in [&[0][..], &[0x80][..], &[1, 0][..], &[1, 0x80][..]] {
            for opcode in [0x9f, 0xa0] {
                assert!(execute(&[opcode], &[nonminimal, &[1]]).is_err());
                assert!(execute(&[opcode], &[&[1], nonminimal]).is_err());
            }
        }
        let five_bytes = [0, 0, 0, 0x80, 0];
        assert!(execute(&[0x9f], &[&five_bytes, &[1]]).is_err());
        assert!(execute(&[0xa0], &[&[1], &five_bytes]).is_err());
        assert!(execute(&[0x9f], &[&[1]]).is_err());
    }

    #[test]
    fn checksig_rejects_bad_encodings_and_nonempty_false_signatures() {
        assert_eq!(execute(&[0xac], &[&[1; 64], &[2; 32]]), Ok(true));
        let mut all = [1; 65];
        assert_eq!(execute(&[0xac], &[&all, &[2; 32]]), Ok(true));
        all[64] = 0;
        assert_eq!(execute(&[0xac], &[&all, &[2; 32]]), Err(Failure::Signature));
        assert_eq!(
            execute(&[0xac], &[&[0; 64], &[2; 32]]),
            Err(Failure::Signature)
        );
        assert_eq!(
            execute(&[0xac], &[&[1; 63], &[2; 32]]),
            Err(Failure::Signature)
        );
        assert_eq!(
            execute(&[0xac], &[&[1; 64], &[2; 33]]),
            Err(Failure::Signature)
        );
        assert_eq!(execute(&[0xac], &[&[], &[2; 32]]), Ok(false));
        assert_eq!(execute(&[0xad, 0x51], &[&[], &[2; 32]]), Ok(false));
        assert_eq!(execute(&[0xad, 0x51], &[&[1; 64], &[2; 32]]), Ok(true));
    }

    #[test]
    fn repeated_nonempty_signatures_exhaust_bip342_budget() {
        let repeated = |count| {
            let mut script = [0x6e, 0xad].repeat(count);
            script.extend_from_slice(&[0x75, 0x75, 0x51]);
            script
        };
        assert_eq!(execute(&repeated(3), &[&[1; 64], &[2; 32]]), Ok(true));
        assert_eq!(
            execute(&repeated(4), &[&[1; 64], &[2; 32]]),
            Err(Failure::Signature)
        );
    }

    #[test]
    fn csv_uses_minimal_five_byte_script_numbers_without_popping() {
        let crypto = TestCrypto::default();
        assert_eq!(
            evaluate(&[0xb2, 1, 20, 0x87], &evidence(&[&[20]]), &crypto),
            Ok(true)
        );
        assert_eq!(&*crypto.sequences.borrow(), &[20]);
        assert_eq!(execute(&[0xb2], &[&[0, 0, 0, 0x80, 0]]), Ok(true));
        assert_eq!(execute(&[0xb2], &[&[19]]), Ok(false));
        assert_eq!(execute(&[0xb2], &[&[0x81]]), Ok(false));
        assert!(execute(&[0xb2], &[&[20, 0]]).is_err());
        assert!(execute(&[0xb2], &[&[0x80]]).is_err());
        assert!(execute(&[0xb2], &[&[0, 0, 0, 0, 0, 1]]).is_err());
    }
}
