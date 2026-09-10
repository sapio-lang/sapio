//! Fresh, bounded WASM execution over the signed transaction projection.

use super::{EvaluationError, SignedTransactionView};
use bitcoin::consensus::Encodable;
use sapio_base::program::{EvaluatorId, WasmVersion, MAX_PARAMETER_BYTES, MAX_PROGRAM_BYTES};
use sapio_wasm::host::{
    add_crypto_imports, add_crypto_imports_v2, bind_crypto, new_evaluator_store, INSTANCE_FUEL,
};
use sapio_wasm::{CRYPTO_NAMESPACE, CRYPTO_NAMESPACE_V2};
use std::ops::Range;
use std::sync::Arc;
use wasmer::wasmparser::{Parser, Payload, TypeRef, ValType};
use wasmer::{Imports, Instance, Module, RuntimeError, Store, TypedFunction};
use wasmer_middlewares::metering::{get_remaining_points, MeteringPoints};

// Pointer/length pairs for program, parameters, signed view and witness.
type EvaluationArguments = (i32, i32, i32, i32, i32, i32, i32, i32);

/// Maximum encoded signed transaction view supplied to a WASM evaluator.
pub const MAX_SIGNED_VIEW_BYTES: usize = 1_048_576;

/// Maximum aggregate parameters and declared locals of defined functions.
///
/// Local counts have a compact binary encoding. This pre-compilation limit
/// bounds their JIT expansion independently of execution fuel and module size.
pub const MAX_FUNCTION_SLOTS: u32 = 65_536;

/// An immutable WASM interpreter whose identity commits its exact module bytes.
///
/// Registration provides code, never a host-language callback. Every request
/// uses a fresh instance with one shared fuel allowance across allocations,
/// evaluation, and native crypto imports. Module validation occurs on execution.
#[derive(Clone, Debug)]
pub struct WasmEvaluator {
    id: EvaluatorId,
    module: Arc<[u8]>,
    version: WasmVersion,
}

impl WasmEvaluator {
    /// Bound binary module bytes and derive their public evaluator identity.
    pub fn new(module: Vec<u8>) -> Result<Self, EvaluationError> {
        Self::with_version(WasmVersion::V1, module)
    }

    /// Commit both a module and its immutable runtime/transaction-view ABI.
    pub fn with_version(version: WasmVersion, module: Vec<u8>) -> Result<Self, EvaluationError> {
        check_module_bytes(&module)?;
        Ok(Self {
            id: EvaluatorId::for_wasm_version(&module, version),
            module: module.into(),
            version,
        })
    }

    /// The runtime version committed by this interpreter's identity.
    pub fn version(&self) -> WasmVersion {
        self.version
    }

    /// The domain-separated hash of the registered interpreter's exact bytes.
    pub fn id(&self) -> EvaluatorId {
        self.id
    }

    /// Borrow the immutable module needed to reproduce evaluator semantics.
    pub fn module(&self) -> &[u8] {
        &self.module
    }
}

fn failure(error: impl std::fmt::Display) -> EvaluationError {
    EvaluationError(error.to_string())
}

fn check_module_bytes(module: &[u8]) -> Result<(), EvaluationError> {
    if module.len() > MAX_PROGRAM_BYTES {
        return Err(failure("WASM evaluator module exceeds 65536 bytes"));
    }
    if !module.starts_with(b"\0asm\x01\0\0\0") {
        return Err(failure(
            "WASM evaluator requires a version-one binary module",
        ));
    }
    Ok(())
}

fn check_compilation_budget(version: WasmVersion, module: &[u8]) -> Result<(), EvaluationError> {
    let mut function_types = Vec::new();
    let mut slots = 0u32;
    let mut add_slots = |count| -> Result<(), EvaluationError> {
        slots = slots
            .checked_add(count)
            .filter(|total| *total <= MAX_FUNCTION_SLOTS)
            .ok_or_else(|| {
                failure("WASM evaluator exceeds 65536 function parameter/local slots")
            })?;
        Ok(())
    };
    for payload in Parser::new(0).parse_all(module) {
        match payload.map_err(failure)? {
            Payload::TypeSection(types) => {
                // Grow only for successfully decoded entries, never reserve
                // using an untrusted section count. The module byte cap
                // bounds this table and wasmparser bounds each function type.
                for ty in types.into_iter_err_on_gc_types() {
                    function_types.push(ty.map_err(failure)?);
                }
            }
            Payload::ImportSection(imports) => {
                for import in imports {
                    let import = import.map_err(failure)?;
                    let expected_params = match (import.module, import.name) {
                        (CRYPTO_NAMESPACE, "sha256" | "schnorr_verify") => 3,
                        (CRYPTO_NAMESPACE, "bip32_derive") => 4,
                        (CRYPTO_NAMESPACE_V2, "schnorr_verify" | "xonly_tweak_check")
                            if version == WasmVersion::V2 =>
                        {
                            4
                        }
                        _ => {
                            return Err(failure(
                                "unsupported WASM evaluator crypto import or namespace",
                            ))
                        }
                    };
                    let TypeRef::Func(index) = import.ty else {
                        return Err(failure("WASM evaluator imports must be crypto functions"));
                    };
                    let ty = function_types
                        .get(index as usize)
                        .ok_or_else(|| failure("WASM evaluator import has an unknown type"))?;
                    if ty.params().len() != expected_params
                        || ty.params().iter().any(|ty| *ty != ValType::I32)
                        || ty.results() != [ValType::I32]
                    {
                        return Err(failure(
                            "WASM evaluator crypto import has an invalid signature",
                        ));
                    }
                }
            }
            Payload::FunctionSection(functions) => {
                for type_index in functions {
                    let ty = function_types
                        .get(type_index.map_err(failure)? as usize)
                        .ok_or_else(|| failure("WASM evaluator function has an unknown type"))?;
                    // A shared type contributes once per defined function,
                    // because each body needs its own parameter slots.
                    add_slots(u32::try_from(ty.params().len()).map_err(failure)?)?;
                }
            }
            Payload::CodeSectionEntry(body) => {
                for local in body.get_locals_reader().map_err(failure)? {
                    add_slots(local.map_err(failure)?.0)?;
                }
            }
            Payload::StartSection { .. } => {
                return Err(failure(
                    "WASM evaluator modules cannot have a start function",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn execution_error(
    store: &mut Store,
    instance: &Instance,
    operation: &str,
    error: RuntimeError,
) -> EvaluationError {
    if get_remaining_points(store, instance) == MeteringPoints::Exhausted {
        failure(format!(
            "WASM evaluator exhausted {INSTANCE_FUEL} fuel points during {operation}"
        ))
    } else {
        failure(format!("WASM evaluator {operation} failed: {error}"))
    }
}

pub(super) fn evaluate(
    version: WasmVersion,
    module: &[u8],
    program: &[u8],
    parameters: &[u8],
    view: &SignedTransactionView<'_>,
    witness: &[u8],
) -> Result<bool, EvaluationError> {
    check_module_bytes(module)?;
    check_compilation_budget(version, module)?;
    if program.len() > MAX_PROGRAM_BYTES
        || parameters.len() > MAX_PARAMETER_BYTES
        || witness.len() > super::MAX_WITNESS_BYTES
    {
        return Err(failure("WASM evaluator input exceeds its byte limit"));
    }
    let encoded_view = encode_view(version, view)?;
    let mut store = new_evaluator_store();
    let module = Module::new(&store, module).map_err(failure)?;
    let mut imports = Imports::new();
    let crypto = match version {
        WasmVersion::V1 => add_crypto_imports(&mut store, &mut imports),
        WasmVersion::V2 => add_crypto_imports_v2(&mut store, &mut imports),
    };
    let instance = Instance::new(&mut store, &module, &imports).map_err(failure)?;
    bind_crypto(&crypto, &mut store, &instance).map_err(failure)?;
    let memory = instance
        .exports
        .get_memory("memory")
        .map_err(failure)?
        .clone();
    let (allocate_name, evaluate_name) = match version {
        WasmVersion::V1 => ("sapio_alloc_v1", "sapio_evaluate_v1"),
        WasmVersion::V2 => ("sapio_alloc_v2", "sapio_evaluate_v2"),
    };
    let allocate: TypedFunction<i32, i32> = instance
        .exports
        .get_typed_function(&store, allocate_name)
        .map_err(failure)?;
    let evaluate: TypedFunction<EvaluationArguments, i32> = instance
        .exports
        .get_typed_function(&store, evaluate_name)
        .map_err(failure)?;
    let inputs = [program, parameters, &encoded_view, witness];
    let mut ranges: [Range<u64>; 4] = std::array::from_fn(|_| 0..0);
    for (index, bytes) in inputs.iter().enumerate() {
        if bytes.is_empty() {
            continue;
        }
        let pointer = allocate
            .call(&mut store, bytes.len() as i32)
            .map_err(|error| execution_error(&mut store, &instance, allocate_name, error))?;
        let start = u64::from(pointer as u32);
        let end = start + bytes.len() as u64;
        if end > memory.view(&store).data_size() {
            return Err(failure(
                "WASM evaluator allocation is outside linear memory",
            ));
        }
        let range = start..end;
        if ranges[..index].iter().any(|previous| {
            !previous.is_empty() && range.start < previous.end && previous.start < range.end
        }) {
            return Err(failure("WASM evaluator input allocations overlap"));
        }
        ranges[index] = range;
    }
    // Allocate all regions before copying any input. An allocator cannot
    // rewrite an earlier input while allocating a later region.
    for (bytes, range) in inputs.iter().zip(&ranges) {
        memory
            .view(&store)
            .write(range.start, bytes)
            .map_err(failure)?;
    }
    let result = evaluate
        .call(
            &mut store,
            ranges[0].start as i32,
            inputs[0].len() as i32,
            ranges[1].start as i32,
            inputs[1].len() as i32,
            ranges[2].start as i32,
            inputs[2].len() as i32,
            ranges[3].start as i32,
            inputs[3].len() as i32,
        )
        .map_err(|error| execution_error(&mut store, &instance, evaluate_name, error))?;
    match result {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(failure("WASM evaluator must return exactly zero or one")),
    }
}

fn encode_view(
    version: WasmVersion,
    view: &SignedTransactionView<'_>,
) -> Result<Vec<u8>, EvaluationError> {
    // Compute the bounded size before allocating. A script can exceed the
    // limit in a native request without forcing a copy of that script here.
    let mut size = 20usize;
    let mut add = |amount: usize| -> Result<(), EvaluationError> {
        size = size
            .checked_add(amount)
            .filter(|size| *size <= MAX_SIGNED_VIEW_BYTES)
            .ok_or_else(|| failure("signed transaction view exceeds 1048576 bytes"))?;
        Ok(())
    };
    for input in view.inputs() {
        add(52)?;
        add(input.prevout.script_pubkey.len())?;
    }
    for output in view.outputs() {
        add(12)?;
        add(output.script_pubkey.len())?;
    }
    if version == WasmVersion::V2 {
        add(36)?;
        add(view.annex().map_or(0, <[u8]>::len))?;
    }
    let input_count = u32::try_from(view.inputs().len()).map_err(failure)?;
    let output_count = u32::try_from(view.outputs().len()).map_err(failure)?;
    let mut bytes = Vec::with_capacity(size);
    bytes.extend_from_slice(&view.version().to_le_bytes());
    bytes.extend_from_slice(&view.lock_time().to_le_bytes());
    bytes.extend_from_slice(&view.input_index().to_le_bytes());
    bytes.extend_from_slice(&input_count.to_le_bytes());
    for input in view.inputs() {
        input
            .previous_output
            .consensus_encode(&mut bytes)
            .expect("encoding an outpoint into a Vec cannot fail");
        bytes.extend_from_slice(&input.sequence.to_le_bytes());
        bytes.extend_from_slice(&input.prevout.value.to_le_bytes());
        bytes.extend_from_slice(&(input.prevout.script_pubkey.len() as u32).to_le_bytes());
        bytes.extend_from_slice(input.prevout.script_pubkey.as_bytes());
    }
    bytes.extend_from_slice(&output_count.to_le_bytes());
    for output in view.outputs() {
        bytes.extend_from_slice(&output.value.to_le_bytes());
        bytes.extend_from_slice(&(output.script_pubkey.len() as u32).to_le_bytes());
        bytes.extend_from_slice(output.script_pubkey.as_bytes());
    }
    if version == WasmVersion::V2 {
        bytes.extend_from_slice(&view.internal_key().serialize());
        let annex = view.annex().unwrap_or_default();
        bytes.extend_from_slice(&(annex.len() as u32).to_le_bytes());
        bytes.extend_from_slice(annex);
    }
    debug_assert_eq!(bytes.len(), size);
    Ok(bytes)
}

#[cfg(test)]
mod tests;
