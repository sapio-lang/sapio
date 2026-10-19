//! Allocation-free execution of the straight-line Tapscript profile used here.
//!
//! Unsupported opcodes, including every OP_SUCCESS opcode, fail closed. This
//! deliberately bounded profile does not claim to be a consensus interpreter.
use crate::Failure;

const MAX_SCRIPT: usize = 10_000;
const MAX_WITNESS: usize = 65_536;
const MAX_ITEMS: usize = 1_000;
const MAX_ELEMENT: usize = 520;

/// Authenticated transaction operations supplied by the evaluator boundary.
pub trait Crypto {
    /// Compute SHA256 over the exact supplied bytes.
    fn hash(&self, bytes: &[u8]) -> Result<[u8; 32], Failure>;
    /// Check a signature against the current transaction's selected leaf.
    fn check_signature(&self, signature: &[u8], key: &[u8]) -> Result<bool, Failure>;
    /// Apply BIP112 to the current input and this nonnegative Script number.
    fn check_sequence(&self, operand: i64) -> Result<bool, Failure>;
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
    ) -> Result<bool, Failure> {
        if script.len() > MAX_SCRIPT {
            return Err(Failure::InvalidEncoding);
        }
        self.load_witness(witness, script.len())?;
        let mut cursor = 0;
        while cursor < script.len() {
            let opcode = take(script, &mut cursor, 1)?[0];
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
                    if !minimal_push(opcode, bytes) {
                        return Err(Failure::InvalidEncoding);
                    }
                    self.push(bytes)?;
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
                0x75 => {
                    self.need(1)?;
                    self.main -= 1;
                }
                0x76 => self.duplicate(1)?,
                0x7b => {
                    self.need(3)?;
                    self.items.swap(self.main - 3, self.main - 2);
                    self.items.swap(self.main - 2, self.main - 1);
                }
                0x7c => {
                    self.need(2)?;
                    self.items.swap(self.main - 2, self.main - 1);
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
                    let sequence = script_number(self.items[self.main - 1].bytes())?;
                    if sequence < 0 || !crypto.check_sequence(sequence)? {
                        return Ok(false);
                    }
                }
                _ => return Err(Failure::InvalidEncoding),
            }
        }
        // Tapscript requires exactly one true main-stack result. The altstack
        // is not part of cleanstack and may retain values, as in Bitcoin Core.
        Ok(self.main == 1 && truth(self.items[0].bytes()))
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

fn script_number(bytes: &[u8]) -> Result<i64, Failure> {
    if bytes.len() > 5 {
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
    #[cfg(target_arch = "wasm32")]
    let machine = unsafe { &mut *core::ptr::addr_of_mut!(MACHINE) };
    #[cfg(not(target_arch = "wasm32"))]
    let mut guard = MACHINE.lock().expect("Script execution arena poisoned");
    #[cfg(not(target_arch = "wasm32"))]
    let machine = &mut *guard;
    machine.run(script, witness, crypto)
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
        script.extend(vec![0x6c, 0x75].repeat(MAX_ITEMS));
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
            let mut script = vec![0x6e, 0xad].repeat(count);
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
