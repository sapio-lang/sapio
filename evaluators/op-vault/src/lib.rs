//! Historical BIP345 leaf-update/recovery semantics for one vault input.
#![no_std]

#[path = "../../v2.rs"]
mod guest;
use core::ptr::addr_of_mut;
use guest::Arguments;
use sapio_covenant_fragments::{vault, Context, Failure, XOnlyKey, MAX_VIEW_BYTES};

static mut SCRATCH: [u8; MAX_VIEW_BYTES] = [0; MAX_VIEW_BYTES];

#[no_mangle]
pub extern "C" fn sapio_evaluate_v2(
    pp: u32,
    pl: u32,
    ap: u32,
    al: u32,
    vp: u32,
    vl: u32,
    wp: u32,
    wl: u32,
) -> i32 {
    guest::evaluate([pp, pl, ap, al, vp, vl, wp, wl], evaluate)
}

fn evaluate(arguments: Arguments<'_>) -> Result<bool, Failure> {
    if !arguments.program.is_empty() {
        return Err(Failure::InvalidEncoding);
    }
    let context = Context::parse_v2(arguments.view)?;
    let principal = context.single_vault_input()?;
    if principal == 0 || principal > 2_100_000_000_000_000 {
        return Err(Failure::InvalidEncoding);
    }
    let scratch = unsafe {
        core::slice::from_raw_parts_mut(addr_of_mut!(SCRATCH).cast::<u8>(), MAX_VIEW_BYTES)
    };
    match arguments.parameters {
        [0, low, high, root @ ..] if root.len() == 78 => {
            let delay = u16::from_le_bytes([*low, *high]);
            let mut witness = Reader::new(arguments.witness);
            let expected = witness
                .take(32)?
                .try_into()
                .map_err(|_| Failure::InvalidEncoding)?;
            let trigger_index = witness.u32()?;
            let revault_index = witness.u32()?;
            let revault_amount = witness.u64()?;
            if revault_amount > principal
                || (revault_amount == 0) != (revault_index == u32::MAX)
                || revault_index == trigger_index
            {
                return Err(Failure::InvalidEncoding);
            }
            let source_length = witness.u32()? as usize;
            let source = witness.take(source_length)?;
            let control_length = witness.u32()? as usize;
            let control = witness.take(control_length)?;
            witness.finish()?;
            let (trigger_amount, trigger_script) = context
                .output(trigger_index)
                .ok_or(Failure::InvalidEncoding)?;
            if trigger_amount < principal - revault_amount
                || trigger_script.len() != 34
                || trigger_script[..2] != [0x51, 32]
            {
                return Ok(false);
            }
            if revault_amount != 0 {
                let (amount, script) = context
                    .output(revault_index)
                    .ok_or(Failure::InvalidEncoding)?;
                let (_, previous_script) = context
                    .input(context.input_index())
                    .ok_or(Failure::InvalidEncoding)?;
                if amount < revault_amount || script != previous_script {
                    return Ok(false);
                }
            }
            let key = vault::ctv_key(
                root.try_into().map_err(|_| Failure::InvalidEncoding)?,
                expected,
                scratch,
            )?;
            let mut script = [0; 40];
            let replacement = vault::delayed_key_leaf(key, delay, &mut script)?;
            vault::verify_leaf_replacement(
                &context,
                source,
                control,
                replacement,
                XOnlyKey::from_slice(&trigger_script[2..])?,
                scratch,
            )?;
            Ok(true)
        }
        [1, expected @ ..] if expected.len() == 32 => {
            let mut witness = Reader::new(arguments.witness);
            let output_index = witness.u32()?;
            witness.finish()?;
            let (amount, script) = context
                .output(output_index)
                .ok_or(Failure::InvalidEncoding)?;
            Ok(amount >= principal && vault::recovery_hash(script, scratch)? == expected)
        }
        _ => Err(Failure::InvalidEncoding),
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    used: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, used: 0 }
    }
    fn take(&mut self, length: usize) -> Result<&'a [u8], Failure> {
        let end = self
            .used
            .checked_add(length)
            .ok_or(Failure::InvalidEncoding)?;
        let bytes = self
            .bytes
            .get(self.used..end)
            .ok_or(Failure::InvalidEncoding)?;
        self.used = end;
        Ok(bytes)
    }
    fn u32(&mut self) -> Result<u32, Failure> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| Failure::InvalidEncoding)?,
        ))
    }
    fn u64(&mut self) -> Result<u64, Failure> {
        Ok(u64::from_le_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| Failure::InvalidEncoding)?,
        ))
    }
    fn finish(self) -> Result<(), Failure> {
        if self.used == self.bytes.len() {
            Ok(())
        } else {
            Err(Failure::InvalidEncoding)
        }
    }
}
