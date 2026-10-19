//! BIP341 DEFAULT/ALL messages reconstructed from authenticated signed fields.

use sapio_covenant_fragments::{Context, Failure};

/// Hash a BIP342 Script spend with no OP_CODESEPARATOR.
///
/// `leaf` must come from the runtime's authenticated wrapper context, never
/// from witness evidence. Only DEFAULT (0) and ALL (1) are supported. The
/// scratch arena must accommodate the context's serialized inputs or outputs;
/// the fragment SDK's `MAX_VIEW_BYTES` always suffices for a parsed context.
pub fn signature_hash(
    context: &Context<'_>,
    leaf: &[u8; 32],
    hash_type: u8,
    scratch: &mut [u8],
    hash: impl Fn(&[u8]) -> Result<[u8; 32], Failure>,
) -> Result<[u8; 32], Failure> {
    if !matches!(hash_type, 0 | 1) {
        return Err(Failure::InvalidEncoding);
    }
    let mut writer = Writer::new(scratch);
    for index in 0..context.input_count() {
        writer.put(context.outpoint(index).ok_or(Failure::InvalidEncoding)?)?;
    }
    let prevouts = hash(writer.written())?;
    writer.reset();
    for index in 0..context.input_count() {
        let (amount, _) = context.input(index).ok_or(Failure::InvalidEncoding)?;
        writer.put(&amount.to_le_bytes())?;
    }
    let amounts = hash(writer.written())?;
    writer.reset();
    for index in 0..context.input_count() {
        let (_, script) = context.input(index).ok_or(Failure::InvalidEncoding)?;
        writer.compact_size(script.len())?;
        writer.put(script)?;
    }
    let scripts = hash(writer.written())?;
    writer.reset();
    for index in 0..context.input_count() {
        let sequence = context.sequence(index).ok_or(Failure::InvalidEncoding)?;
        writer.put(&sequence.to_le_bytes())?;
    }
    let sequences = hash(writer.written())?;
    writer.reset();
    for index in 0..context.output_count() {
        let (amount, script) = context.output(index).ok_or(Failure::InvalidEncoding)?;
        writer.put(&amount.to_le_bytes())?;
        writer.compact_size(script.len())?;
        writer.put(script)?;
    }
    let outputs = hash(writer.written())?;
    writer.reset();
    let annex = if context.annex().is_empty() {
        None
    } else {
        writer.compact_size(context.annex().len())?;
        writer.put(context.annex())?;
        Some(hash(writer.written())?)
    };

    let tag = hash(b"TapSighash")?;
    let mut message = [0; 308];
    let mut writer = Writer::new(&mut message);
    writer.put(&tag)?;
    writer.put(&tag)?;
    writer.put(&[0, hash_type])?;
    writer.put(&context.transaction_version().to_le_bytes())?;
    writer.put(&context.lock_time().to_le_bytes())?;
    writer.put(&prevouts)?;
    writer.put(&amounts)?;
    writer.put(&scripts)?;
    writer.put(&sequences)?;
    writer.put(&outputs)?;
    writer.put(&[2 | u8::from(annex.is_some())])?;
    writer.put(&context.input_index().to_le_bytes())?;
    if let Some(annex) = annex {
        writer.put(&annex)?;
    }
    writer.put(leaf)?;
    writer.put(&[0])?;
    writer.put(&u32::MAX.to_le_bytes())?;
    hash(writer.written())
}

struct Writer<'a> {
    bytes: &'a mut [u8],
    position: usize,
}

impl<'a> Writer<'a> {
    fn new(bytes: &'a mut [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn reset(&mut self) {
        self.position = 0;
    }

    fn put(&mut self, bytes: &[u8]) -> Result<(), Failure> {
        let end = self
            .position
            .checked_add(bytes.len())
            .ok_or(Failure::ScratchTooSmall)?;
        self.bytes
            .get_mut(self.position..end)
            .ok_or(Failure::ScratchTooSmall)?
            .copy_from_slice(bytes);
        self.position = end;
        Ok(())
    }

    fn compact_size(&mut self, value: usize) -> Result<(), Failure> {
        match value {
            0..=252 => self.put(&[value as u8]),
            253..=65_535 => {
                self.put(&[253])?;
                self.put(&(value as u16).to_le_bytes())
            }
            _ => {
                self.put(&[254])?;
                self.put(
                    &u32::try_from(value)
                        .map_err(|_| Failure::InvalidEncoding)?
                        .to_le_bytes(),
                )
            }
        }
    }

    fn written(&self) -> &[u8] {
        &self.bytes[..self.position]
    }
}
