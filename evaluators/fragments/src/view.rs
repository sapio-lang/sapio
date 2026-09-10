use crate::{crypto, Failure, TemplateHash, XOnlyKey, MAX_VIEW_BYTES};

/// A completely decoded view supplied by the WASM-v2 signing runtime.
///
/// The runtime authenticates the physical internal key and annex before
/// invoking a guest. Parsing arbitrary application bytes does not establish
/// that provenance. The selected prevout must be a P2TR output.
pub struct Context<'a> {
    version: &'a [u8],
    lock_time: &'a [u8],
    input_index: u32,
    input_count: u32,
    inputs: &'a [u8],
    output_count: u32,
    outputs: &'a [u8],
    internal_key: XOnlyKey,
    output_key: XOnlyKey,
    annex: &'a [u8],
}

impl<'a> Context<'a> {
    /// Decode the complete v1 projection followed by internal-key/annex data.
    pub fn parse_v2(bytes: &'a [u8]) -> Result<Self, Failure> {
        if bytes.len() > MAX_VIEW_BYTES {
            return Err(Failure::InvalidEncoding);
        }
        let mut reader = Reader::new(bytes);
        let version = reader.take(4)?;
        let lock_time = reader.take(4)?;
        let input_index = reader.u32()?;
        let input_count = reader.u32()?;
        if input_index >= input_count {
            return Err(Failure::InvalidEncoding);
        }
        let inputs_start = reader.position;
        let mut selected_script = &[][..];
        for index in 0..input_count {
            reader.take(48)?;
            let length = reader.u32()?;
            let script = reader.take(length as usize)?;
            if index == input_index {
                selected_script = script;
            }
        }
        if selected_script.len() != 34 || selected_script[..2] != [0x51, 0x20] {
            return Err(Failure::InvalidEncoding);
        }
        let output_key = XOnlyKey::from_slice(&selected_script[2..])?;
        let inputs = &bytes[inputs_start..reader.position];
        let output_count = reader.u32()?;
        let outputs_start = reader.position;
        for _ in 0..output_count {
            reader.take(8)?;
            let length = reader.u32()?;
            reader.take(length as usize)?;
        }
        let outputs = &bytes[outputs_start..reader.position];
        let internal_key = XOnlyKey::from_slice(reader.take(32)?)?;
        let length = reader.u32()?;
        let annex = reader.take(length as usize)?;
        if reader.position != bytes.len() || (!annex.is_empty() && annex[0] != 0x50) {
            return Err(Failure::InvalidEncoding);
        }
        Ok(Self {
            version,
            lock_time,
            input_index,
            input_count,
            inputs,
            output_count,
            outputs,
            internal_key,
            output_key,
            annex,
        })
    }

    /// The selected input's authenticated physical Taproot internal key.
    pub fn internal_key(&self) -> XOnlyKey {
        self.internal_key
    }

    /// The x-only P2TR output key of the selected input's authenticated prevout.
    pub fn output_key(&self) -> XOnlyKey {
        self.output_key
    }

    /// The selected input's raw annex, including its 0x50 prefix when present.
    pub fn annex(&self) -> &[u8] {
        self.annex
    }

    /// Compute the BIP446 TemplateHash using caller-owned scratch storage.
    ///
    /// `MAX_VIEW_BYTES` of scratch suffices for every accepted view. Sequence,
    /// output, and annex serializations reuse this buffer and are hashed by
    /// the generic SHA256 import. No native-witness restriction is imposed on
    /// auxiliary inputs: TemplateHash does not commit any scriptSig.
    pub fn template_hash(&self, scratch: &mut [u8]) -> Result<TemplateHash, Failure> {
        let mut writer = Writer {
            bytes: scratch,
            position: 0,
        };
        let mut inputs = Reader::new(self.inputs);
        for _ in 0..self.input_count {
            inputs.take(36)?;
            writer.put(inputs.take(4)?)?;
            inputs.take(8)?;
            let length = inputs.u32()?;
            inputs.take(length as usize)?;
        }
        let sequences = crypto::hash(writer.written())?;
        writer.position = 0;
        let mut outputs = Reader::new(self.outputs);
        for _ in 0..self.output_count {
            writer.put(outputs.take(8)?)?;
            let length = outputs.u32()?;
            writer.compact_size(length)?;
            writer.put(outputs.take(length as usize)?)?;
        }
        let outputs = crypto::hash(writer.written())?;
        writer.position = 0;
        let annex_hash = if self.annex.is_empty() {
            None
        } else {
            writer.compact_size(self.annex.len() as u32)?;
            writer.put(self.annex)?;
            Some(crypto::hash(writer.written())?)
        };

        // BIP340 tagged hashing of the 77-byte or 109-byte BIP446 message.
        let tag = crypto::hash(b"TemplateHash")?;
        let mut tagged = [0; 173];
        let mut writer = Writer {
            bytes: &mut tagged,
            position: 0,
        };
        writer.put(&tag)?;
        writer.put(&tag)?;
        writer.put(self.version)?;
        writer.put(self.lock_time)?;
        writer.put(&sequences)?;
        writer.put(&outputs)?;
        writer.put(&[u8::from(annex_hash.is_some())])?;
        writer.put(&self.input_index.to_le_bytes())?;
        if let Some(annex_hash) = annex_hash {
            writer.put(&annex_hash)?;
        }
        crypto::hash(writer.written()).map(TemplateHash)
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], Failure> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(Failure::InvalidEncoding)?;
        let result = self
            .bytes
            .get(self.position..end)
            .ok_or(Failure::InvalidEncoding)?;
        self.position = end;
        Ok(result)
    }

    fn u32(&mut self) -> Result<u32, Failure> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| Failure::InvalidEncoding)?,
        ))
    }
}

struct Writer<'a> {
    bytes: &'a mut [u8],
    position: usize,
}

impl Writer<'_> {
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

    fn compact_size(&mut self, length: u32) -> Result<(), Failure> {
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

    fn written(&self) -> &[u8] {
        &self.bytes[..self.position]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    fn encoded_view() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2_i32.to_le_bytes());
        bytes.extend_from_slice(&20_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        let mut taproot = std::vec![0x51, 0x20];
        taproot.extend_from_slice(&[3; 32]);
        for script in [&[0x76, 0xa9, 0x14][..], &taproot] {
            bytes.extend_from_slice(&[0; 48]);
            bytes.extend_from_slice(&(script.len() as u32).to_le_bytes());
            bytes.extend_from_slice(script);
        }
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&500_u64.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.push(0x6a);
        bytes.extend_from_slice(&[4; 32]);
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&[0x50, 7]);
        bytes
    }

    #[test]
    fn v2_projection_preserves_distinct_internal_and_output_keys() {
        let encoded = encoded_view();
        let context = Context::parse_v2(&encoded).unwrap();
        assert_eq!(context.internal_key().as_bytes(), &[4; 32]);
        assert_eq!(context.output_key().as_bytes(), &[3; 32]);
        assert_eq!(context.annex(), &[0x50, 7]);
        assert_eq!(context.input_index, 1);
    }

    #[test]
    fn v2_projection_rejects_truncation_trailing_data_and_wrong_annex_prefix() {
        let encoded = encoded_view();
        for length in 0..encoded.len() {
            assert!(Context::parse_v2(&encoded[..length]).is_err());
        }
        let mut trailing = encoded.clone();
        trailing.push(0);
        assert!(Context::parse_v2(&trailing).is_err());
        let mut annex = encoded.clone();
        let prefix = annex.len() - 2;
        annex[prefix] = 0x51;
        assert!(Context::parse_v2(&annex).is_err());
        let mut index = encoded;
        index[8..12].copy_from_slice(&2_u32.to_le_bytes());
        assert!(Context::parse_v2(&index).is_err());
    }

    #[test]
    fn compact_size_is_canonical_at_both_multibyte_boundaries() {
        let mut buffer = [0; 5];
        for (value, expected) in [
            (252, &[252][..]),
            (253, &[253, 253, 0][..]),
            (65_535, &[253, 255, 255][..]),
            (65_536, &[254, 0, 0, 1, 0][..]),
        ] {
            let mut writer = Writer {
                bytes: &mut buffer,
                position: 0,
            };
            writer.compact_size(value).unwrap();
            assert_eq!(writer.written(), expected);
        }
        let mut writer = Writer {
            bytes: &mut buffer[..4],
            position: 0,
        };
        assert_eq!(writer.compact_size(65_536), Err(Failure::ScratchTooSmall));
    }
}
