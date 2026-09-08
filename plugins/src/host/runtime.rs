//! Execution limits applied before instantiation, including WASM start code.

use std::sync::{Arc, OnceLock};
use wasmer::sys::{
    BaseTunables, CompilerConfig, Cranelift, CraneliftOptLevel, EngineBuilder, Features,
    FunctionMiddleware, MiddlewareError, MiddlewareReaderState, ModuleMiddleware,
};
use wasmer::wasmparser::{BlockType, Operator};
use wasmer::{ExportIndex, GlobalInit, GlobalType, LocalFunctionIndex, Mutability, Store, Type};
use wasmer_middlewares::Metering;
use wasmer_types::ModuleInfo;

/// Nonrenewable fuel for one instance, shared by start and every exported call.
pub(crate) const INSTANCE_FUEL: u64 = 100_000_000;
/// Accessible linear memory, in 64 KiB WASM pages.
const MAX_MEMORY_PAGES: u32 = 1024;
const MAX_TABLE_ELEMENTS: u32 = 65_536;

pub(super) fn new_store() -> Store {
    store_with_fuel(INSTANCE_FUEL)
}

fn store_with_fuel(fuel: u64) -> Store {
    // Metering carries module-specific global indexes. Each compiler is used
    // for one module; fresh instances may safely share that compiled module.
    let mut compiler = Cranelift::default();
    // Contracts are short-lived; avoid optimizing the large Rust runtime on
    // every source load before executing the small requested compilation.
    compiler.opt_level(CraneliftOptLevel::None);
    compiler.push_middleware(Arc::new(Metering::new(fuel, |_| 1)));
    compiler.push_middleware(Arc::new(ResourceLimits::default()));
    let mut features = Features::new();
    // Atomic waits can block inside native runtime code without spending fuel.
    features.threads(false).memory64(false).multi_memory(false);
    let mut engine = EngineBuilder::new(compiler)
        .set_features(Some(features))
        .engine();
    engine.set_tunables(BaseTunables::for_target(engine.target()));
    Store::new(engine)
}

#[derive(Clone, Copy, Debug)]
struct FuelGlobals {
    remaining: u32,
    exhausted: u32,
    count: u32,
}

#[derive(Debug, Default)]
struct ResourceLimits {
    globals: OnceLock<FuelGlobals>,
}

fn limit_error(message: &str) -> MiddlewareError {
    MiddlewareError::new("sapio-resource-limits", message)
}

impl ModuleMiddleware for ResourceLimits {
    fn transform_module_info(&self, module: &mut ModuleInfo) -> Result<(), MiddlewareError> {
        if module.memories.len() > 1 || module.tables.len() > 1 {
            return Err(limit_error(
                "WASM supports at most one memory and one table",
            ));
        }
        for memory in module.memories.values_mut() {
            if memory.shared || memory.minimum.0 > MAX_MEMORY_PAGES {
                return Err(limit_error("WASM memory exceeds the 64 MiB limit"));
            }
            memory.maximum = Some(
                memory
                    .maximum
                    .unwrap_or(MAX_MEMORY_PAGES.into())
                    .min(MAX_MEMORY_PAGES.into()),
            );
        }
        for table in module.tables.values_mut() {
            if table.minimum > MAX_TABLE_ELEMENTS {
                return Err(limit_error("WASM table exceeds the 65536 element limit"));
            }
            table.maximum = Some(
                table
                    .maximum
                    .unwrap_or(MAX_TABLE_ELEMENTS)
                    .min(MAX_TABLE_ELEMENTS),
            );
        }
        let global = |name: &str| match module.exports.get(name) {
            Some(ExportIndex::Global(index)) => Ok(index.as_u32()),
            _ => Err(limit_error("WASM metering globals are missing")),
        };
        let remaining = global("wasmer_metering_remaining_points")?;
        let exhausted = global("wasmer_metering_points_exhausted")?;
        // Original bytecode is validated before middleware appends globals.
        // Guests cannot address this scratch index or the metering indexes.
        let count = module
            .globals
            .push(GlobalType::new(Type::I32, Mutability::Var));
        module.global_initializers.push(GlobalInit::I32Const(0));
        self.globals
            .set(FuelGlobals {
                remaining,
                exhausted,
                count: count.as_u32(),
            })
            .map_err(|_| limit_error("WASM compiler cannot be reused for another module"))
    }

    fn generate_function_middleware(&self, _: LocalFunctionIndex) -> Box<dyn FunctionMiddleware> {
        Box::new(BulkMetering(
            *self.globals.get().expect("module limits initialized"),
        ))
    }
}

#[derive(Debug)]
struct BulkMetering(FuelGlobals);

impl FunctionMiddleware for BulkMetering {
    fn feed<'a>(
        &mut self,
        operator: Operator<'a>,
        state: &mut MiddlewareReaderState<'a>,
    ) -> Result<(), MiddlewareError> {
        use Operator::*;
        let unit = match operator {
            MemoryCopy { .. } | MemoryFill { .. } | MemoryInit { .. } => 1,
            MemoryGrow { .. } => 65_536,
            TableCopy { .. } | TableFill { .. } | TableInit { .. } | TableGrow { .. } => 16,
            _ => {
                state.push_operator(operator);
                return Ok(());
            }
        };
        let FuelGlobals {
            remaining,
            exhausted,
            count,
        } = self.0;
        // Each bulk operation's top operand is its unsigned i32 count. Save
        // and restore it without a guest call between, then charge in i64 so
        // even u32::MAX pages cannot overflow. Charge before touching memory.
        state.extend([
            GlobalSet {
                global_index: count,
            },
            GlobalGet {
                global_index: remaining,
            },
            GlobalGet {
                global_index: count,
            },
            I64ExtendI32U,
            I64Const { value: unit },
            I64Mul,
            I64LtU,
            If {
                blockty: BlockType::Empty,
            },
            I32Const { value: 1 },
            GlobalSet {
                global_index: exhausted,
            },
            I64Const { value: 0 },
            GlobalSet {
                global_index: remaining,
            },
            Unreachable,
            End,
            GlobalGet {
                global_index: remaining,
            },
            GlobalGet {
                global_index: count,
            },
            I64ExtendI32U,
            I64Const { value: unit },
            I64Mul,
            I64Sub,
            GlobalSet {
                global_index: remaining,
            },
            GlobalGet {
                global_index: count,
            },
        ]);
        state.push_operator(operator);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
