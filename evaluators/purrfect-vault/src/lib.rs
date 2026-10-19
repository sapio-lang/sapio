//! Purrfect vault transaction predicates over the authenticated WASM-v2 view.
//!
//! Parameters contain exactly one stage byte: trigger (0), cancel (1), or
//! complete (2). Trigger and cancel take no evidence. Completion takes the
//! exact stripped trigger transaction, whose txid authenticates its marker.
//! Owner authorization and CSV remain native policy guards. This evaluator
//! additionally checks CSV's transaction constraints before authorizing it.
#![no_std]

#[cfg(test)]
extern crate std;

#[cfg(target_arch = "wasm32")]
#[path = "../../v2.rs"]
mod guest;

use sapio_covenant_fragments::{Context, Failure};

const MARKER_SATS: u64 = 546;
// Upstream reconstructs this complete preimage with OP_CAT, whose result must
// fit one 520-byte Script element. Witness serialization is deliberately absent.
const MAX_TRIGGER_BYTES: usize = 520;

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "sapio_crypto_v1")]
extern "C" {
    fn sha256(pointer: u32, length: u32, output: u32) -> i32;
}

#[cfg(target_arch = "wasm32")]
fn hash(bytes: &[u8]) -> Result<[u8; 32], Failure> {
    let mut digest = [0; 32];
    let status = unsafe {
        sha256(
            bytes.as_ptr() as u32,
            bytes.len() as u32,
            digest.as_mut_ptr() as u32,
        )
    };
    match status {
        0 => Ok(digest),
        _ => Err(Failure::Crypto),
    }
}

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
            evaluate(&context, arguments.parameters, arguments.witness, hash)
        },
    )
}

fn evaluate(
    context: &Context<'_>,
    parameters: &[u8],
    witness: &[u8],
    hash: impl Fn(&[u8]) -> Result<[u8; 32], Failure>,
) -> Result<bool, Failure> {
    let [stage @ 0..=2] = parameters else {
        return Err(Failure::InvalidEncoding);
    };
    if context.input_index() != 0
        || context.input_count() != 2
        || !context.annex().is_empty()
        || context.output_count() != if *stage == 0 { 2 } else { 1 }
    {
        return Ok(false);
    }
    let principal = context.single_vault_input()?;
    let (_, vault_script) = context.input(0).ok_or(Failure::InvalidEncoding)?;
    let (sponsor, _) = context.input(1).ok_or(Failure::InvalidEncoding)?;
    let available = principal
        .checked_add(sponsor)
        .ok_or(Failure::InvalidEncoding)?;
    let (paid, destination) = context.output(0).ok_or(Failure::InvalidEncoding)?;
    if paid != principal {
        return Ok(false);
    }
    match *stage {
        0 => {
            if !witness.is_empty() {
                return Err(Failure::InvalidEncoding);
            }
            let (marker, target) = context.output(1).ok_or(Failure::InvalidEncoding)?;
            // Native CAT hashes both serialized outputs from one stack item.
            // The selected vault is P2TR, so its serialized output is 43 bytes.
            let target_prefix = match target.len() {
                0..=252 => 1,
                253..=65_535 => 3,
                _ => 5,
            };
            Ok(destination == vault_script
                && marker == MARKER_SATS
                && 43 + 8 + target_prefix + target.len() <= 520
                && principal
                    .checked_add(marker)
                    .is_some_and(|outputs| outputs <= available))
        }
        1 => {
            if !witness.is_empty() {
                return Err(Failure::InvalidEncoding);
            }
            Ok(destination == vault_script && principal <= available)
        }
        _ => {
            let sequence = context.sequence(0).ok_or(Failure::InvalidEncoding)?;
            if context.transaction_version() < 2
                || sequence & ((1 << 31) | (1 << 22)) != 0
                || sequence & 0xffff < 20
            {
                return Ok(false);
            }
            let trigger = Trigger::parse(witness)?;
            let outpoint = context.outpoint(0).ok_or(Failure::InvalidEncoding)?;
            let txid = hash(&hash(witness)?)?;
            Ok(outpoint[..32] == txid
                && outpoint[32..] == [0; 4]
                && trigger.principal == principal
                && trigger.vault_script == vault_script
                && trigger.destination == destination
                && principal <= available)
        }
    }
}

struct Trigger<'a> {
    principal: u64,
    vault_script: &'a [u8],
    destination: &'a [u8],
}

impl<'a> Trigger<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self, Failure> {
        if bytes.len() > MAX_TRIGGER_BYTES {
            return Err(Failure::InvalidEncoding);
        }
        let mut reader = Reader { bytes, position: 0 };
        reader.take(4)?;
        if reader.compact_size()? != 2 {
            return Err(Failure::InvalidEncoding);
        }
        // The example profile has one P2TR vault and one native witness-v0
        // sponsor. Their scriptSigs are empty by consensus. Parsing this exact
        // shape also excludes segwit marker/flag bytes from the txid preimage.
        for _ in 0..2 {
            reader.take(36)?;
            if reader.compact_size()? != 0 {
                return Err(Failure::InvalidEncoding);
            }
            reader.take(4)?;
        }
        if reader.compact_size()? != 2 {
            return Err(Failure::InvalidEncoding);
        }
        let principal = reader.amount()?;
        let vault_script = reader.script()?;
        if reader.amount()? != MARKER_SATS {
            return Err(Failure::InvalidEncoding);
        }
        let destination = reader.script()?;
        reader.take(4)?;
        if reader.position != bytes.len() {
            return Err(Failure::InvalidEncoding);
        }
        Ok(Self {
            principal,
            vault_script,
            destination,
        })
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], Failure> {
        let end = self
            .position
            .checked_add(count)
            .ok_or(Failure::InvalidEncoding)?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or(Failure::InvalidEncoding)?;
        self.position = end;
        Ok(bytes)
    }

    fn amount(&mut self) -> Result<u64, Failure> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn compact_size(&mut self) -> Result<usize, Failure> {
        let first = self.take(1)?[0];
        let (value, minimum) = match first {
            0..=252 => (u64::from(first), 0),
            253 => (
                u64::from(u16::from_le_bytes(self.take(2)?.try_into().unwrap())),
                253,
            ),
            254 => (
                u64::from(u32::from_le_bytes(self.take(4)?.try_into().unwrap())),
                65_536,
            ),
            255 => (self.amount()?, 0x1_0000_0000),
        };
        if value < minimum {
            return Err(Failure::InvalidEncoding);
        }
        value.try_into().map_err(|_| Failure::InvalidEncoding)
    }

    fn script(&mut self) -> Result<&'a [u8], Failure> {
        let length = self.compact_size()?;
        self.take(length)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    fn trigger() -> Vec<u8> {
        let mut bytes = 2_i32.to_le_bytes().to_vec();
        bytes.push(2);
        for tag in [1, 2] {
            bytes.extend_from_slice(&[tag; 36]);
            bytes.push(0);
            bytes.extend_from_slice(&0xffff_fffd_u32.to_le_bytes());
        }
        bytes.push(2);
        bytes.extend_from_slice(&100_000_u64.to_le_bytes());
        bytes.push(34);
        bytes.extend_from_slice(&[0x51, 0x20]);
        bytes.extend_from_slice(&[3; 32]);
        bytes.extend_from_slice(&MARKER_SATS.to_le_bytes());
        bytes.push(22);
        bytes.extend_from_slice(&[0, 20]);
        bytes.extend_from_slice(&[4; 20]);
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes
    }

    fn view(stage: u8, sponsor: u64) -> Vec<u8> {
        let mut bytes = 2_i32.to_le_bytes().to_vec();
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        let vault = [&[0x51, 0x20][..], &[3; 32]].concat();
        let fee = [&[0, 20][..], &[4; 20]].concat();
        for (tag, amount, script) in [(1, 100_000_u64, &vault), (2, sponsor, &fee)] {
            bytes.extend_from_slice(&[tag; 36]);
            bytes.extend_from_slice(&20_u32.to_le_bytes());
            bytes.extend_from_slice(&amount.to_le_bytes());
            bytes.extend_from_slice(&(script.len() as u32).to_le_bytes());
            bytes.extend_from_slice(script);
        }
        bytes.extend_from_slice(&(if stage == 0 { 2_u32 } else { 1 }).to_le_bytes());
        bytes.extend_from_slice(&100_000_u64.to_le_bytes());
        bytes.extend_from_slice(&(vault.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&vault);
        if stage == 0 {
            bytes.extend_from_slice(&MARKER_SATS.to_le_bytes());
            bytes.extend_from_slice(&(fee.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&fee);
        }
        bytes.extend_from_slice(&[7; 32]);
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes
    }

    #[test]
    fn trigger_and_cancel_preserve_principal_with_an_independent_sponsor() {
        for stage in [0, 1] {
            let encoded = view(stage, 1_546);
            let context = Context::parse_v2(&encoded).unwrap();
            assert_eq!(
                evaluate(&context, &[stage], &[], |_| Err(Failure::Crypto)),
                Ok(true),
            );
            assert_eq!(
                evaluate(&context, &[stage], &[0], |_| Err(Failure::Crypto)),
                Err(Failure::InvalidEncoding),
            );
        }
        let encoded = view(0, MARKER_SATS - 1);
        let context = Context::parse_v2(&encoded).unwrap();
        assert_eq!(
            evaluate(&context, &[0], &[], |_| Err(Failure::Crypto)),
            Ok(false),
        );
    }

    #[test]
    fn stripped_trigger_preserves_both_outputs_and_arbitrary_destination_type() {
        let encoded = trigger();
        let decoded = Trigger::parse(&encoded).unwrap();
        assert_eq!(decoded.principal, 100_000);
        assert_eq!(decoded.vault_script.len(), 34);
        assert_eq!(decoded.destination[..2], [0, 20]);
        assert_eq!(decoded.destination.len(), 22);
    }

    #[test]
    fn trigger_proof_rejects_truncation_trailing_bytes_and_wrong_marker() {
        let encoded = trigger();
        for length in 0..encoded.len() {
            assert!(Trigger::parse(&encoded[..length]).is_err());
        }
        let mut trailing = encoded.clone();
        trailing.push(0);
        assert!(Trigger::parse(&trailing).is_err());
        let mut wrong_marker = encoded;
        wrong_marker[131] ^= 1;
        assert!(Trigger::parse(&wrong_marker).is_err());
    }

    #[test]
    fn trigger_proof_rejects_noncanonical_counts_scriptsigs_and_witness_envelopes() {
        let encoded = trigger();
        let mut noncanonical = encoded.clone();
        noncanonical.splice(4..5, [253, 2, 0]);
        assert!(Trigger::parse(&noncanonical).is_err());
        let mut script_sig = encoded.clone();
        script_sig.splice(41..42, [1, 0]);
        assert!(Trigger::parse(&script_sig).is_err());
        let mut witness = encoded;
        witness.splice(4..4, [0, 1]);
        assert!(Trigger::parse(&witness).is_err());
    }

    #[test]
    fn trigger_proof_bounds_the_complete_cat_preimage() {
        let mut encoded = trigger();
        encoded.resize(MAX_TRIGGER_BYTES + 1, 0);
        assert!(Trigger::parse(&encoded).is_err());
    }
}
