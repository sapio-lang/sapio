//! Bounds for the v1 ABI's buffers and null-terminated strings.
use wasmer::{MemoryView, RuntimeError};

/// Maximum size of one ABI message, including a string's terminator.
pub(super) const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

pub(super) fn runtime_error(error: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::new(error.to_string())
}

pub(super) fn check_length(len: usize) -> Result<(), RuntimeError> {
    if len > MAX_MESSAGE_BYTES {
        return Err(runtime_error("WASM ABI message exceeds 16 MiB"));
    }
    Ok(())
}

fn check_range(memory: &MemoryView<'_>, ptr: i32, len: usize) -> Result<u64, RuntimeError> {
    check_length(len)?;
    // Wasm32 pointers are unsigned, even though the ABI represents them as i32.
    let start = u64::from(ptr as u32);
    if start + len as u64 > memory.data_size() {
        return Err(runtime_error("WASM ABI buffer is outside guest memory"));
    }
    Ok(start)
}

pub(super) fn read_buffer(
    memory: &MemoryView<'_>,
    ptr: i32,
    len: usize,
) -> Result<Vec<u8>, RuntimeError> {
    let start = check_range(memory, ptr, len)?;
    let mut bytes = vec![0; len];
    memory.read(start, &mut bytes).map_err(runtime_error)?;
    Ok(bytes)
}

pub(super) fn write_buffer(
    memory: &MemoryView<'_>,
    ptr: i32,
    bytes: &[u8],
) -> Result<(), RuntimeError> {
    let start = check_range(memory, ptr, bytes.len())?;
    memory.write(start, bytes).map_err(runtime_error)
}

pub(super) fn write_string(
    memory: &MemoryView<'_>,
    ptr: i32,
    value: &str,
) -> Result<(), RuntimeError> {
    if ptr == 0 || value.as_bytes().contains(&0) {
        return Err(runtime_error("WASM ABI requires a non-null C string"));
    }
    check_length(value.len())?;
    let start = check_range(memory, ptr, value.len() + 1)?;
    memory
        .write(start, value.as_bytes())
        .map_err(runtime_error)?;
    memory
        .write_u8(start + value.len() as u64, 0)
        .map_err(runtime_error)
}

pub(super) fn read_string(memory: &MemoryView<'_>, ptr: i32) -> Result<Vec<u8>, RuntimeError> {
    if ptr == 0 {
        return Err(runtime_error("WASM module returned a null string"));
    }
    let start = check_range(memory, ptr, 1)?;
    let limit = (memory.data_size() - start).min(MAX_MESSAGE_BYTES as u64);
    let mut bytes = Vec::new();
    let mut chunk = [0; 4096];
    while (bytes.len() as u64) < limit {
        let count = (limit - bytes.len() as u64).min(chunk.len() as u64) as usize;
        memory
            .read(start + bytes.len() as u64, &mut chunk[..count])
            .map_err(runtime_error)?;
        if let Some(end) = chunk[..count].iter().position(|byte| *byte == 0) {
            bytes.extend_from_slice(&chunk[..end]);
            return Ok(bytes);
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    Err(runtime_error(
        "WASM ABI string is unterminated or exceeds 16 MiB",
    ))
}
