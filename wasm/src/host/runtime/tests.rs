use super::*;
use wasmer::{imports, CompileError, Instance, Module, Value};
use wasmer_middlewares::metering::{get_remaining_points, MeteringPoints};

#[test]
fn evaluator_failed_growth_traps_before_guest_observation() {
    for evaluator in [false, true] {
        let mut store = if evaluator {
            new_evaluator_store()
        } else {
            new_store()
        };
        let module = Module::new(
            &store,
            r#"(module
            (memory 1 2)
            (table 1 2 funcref)
            (func (export "memory") (result i32)
                i32.const 1 memory.grow)
            (func (export "table") (result i32)
                ref.null func i32.const 1 table.grow))"#,
        )
        .unwrap();
        let instance = Instance::new(&mut store, &module, &imports! {}).unwrap();
        for name in ["memory", "table"] {
            let grow = instance
                .exports
                .get_typed_function::<(), i32>(&store, name)
                .unwrap();
            assert_eq!(grow.call(&mut store).unwrap(), 1);
            if evaluator {
                assert!(grow.call(&mut store).is_err());
                assert!(matches!(
                    get_remaining_points(&mut store, &instance),
                    MeteringPoints::Remaining(_)
                ));
            } else {
                assert_eq!(grow.call(&mut store).unwrap(), -1);
            }
        }
    }
}

#[test]
fn evaluator_arithmetic_nans_are_canonical_and_simd_is_disabled() {
    let mut store = new_evaluator_store();
    let module = Module::new(
        &store,
        r#"(module
        (func (export "f32") (result i32)
            f32.const -1 f32.sqrt i32.reinterpret_f32)
        (func (export "f64") (result i64)
            f64.const -1 f64.sqrt i64.reinterpret_f64))"#,
    )
    .unwrap();
    let instance = Instance::new(&mut store, &module, &imports! {}).unwrap();
    assert_eq!(
        instance
            .exports
            .get_typed_function::<(), i32>(&store, "f32")
            .unwrap()
            .call(&mut store)
            .unwrap(),
        0x7fc0_0000
    );
    assert_eq!(
        instance
            .exports
            .get_typed_function::<(), i64>(&store, "f64")
            .unwrap()
            .call(&mut store)
            .unwrap(),
        0x7ff8_0000_0000_0000
    );
    let store = new_evaluator_store();
    assert!(Module::new(
        &store,
        "(module (func (result v128) v128.const i32x4 0 0 0 0))"
    )
    .is_err());
}

fn instance(source: &str, fuel: u64) -> (Store, Instance) {
    let mut store = store_with_fuel(fuel);
    let module = Module::new(&store, source).unwrap();
    let instance = Instance::new(&mut store, &module, &imports! {}).unwrap();
    (store, instance)
}

fn run(instance: &Instance, store: &mut Store, count: i32) -> Result<i32, wasmer::RuntimeError> {
    instance
        .exports
        .get_typed_function::<i32, i32>(store, "run")
        .unwrap()
        .call(store, count)
}

fn remaining(instance: &Instance, store: &mut Store) -> u64 {
    match get_remaining_points(store, instance) {
        MeteringPoints::Remaining(fuel) => fuel,
        MeteringPoints::Exhausted => panic!("unexpected fuel exhaustion"),
    }
}

#[test]
fn oversized_initial_resources_and_multiple_tables_fail_before_instantiation() {
    for (source, reason) in [
        ("(module (memory 1025))", "64 MiB"),
        ("(module (table 65537 funcref))", "65536 element"),
        (
            "(module (table 1 funcref) (table 1 funcref))",
            "at most one",
        ),
    ] {
        let store = store_with_fuel(1_000_000);
        let error = Module::new(&store, source).unwrap_err();
        assert!(
            matches!(error, CompileError::MiddlewareError(_)),
            "{source}: {error}"
        );
        assert!(error.to_string().contains(reason), "{source}: {error}");
    }
}

#[test]
fn memory_growth_obeys_host_cap_and_smaller_declared_maximum() {
    for (limits, maximum) in [("0", 1024), ("0 65536", 1024), ("0 2", 2)] {
        let source = format!(
            "(module (memory (export \"memory\") {limits})
                (func (export \"run\") (param i32) (result i32)
                    local.get 0 memory.grow))"
        );
        let (mut store, instance) = instance(&source, 1 << 40);
        let memory = instance.exports.get_memory("memory").unwrap();
        assert_eq!(memory.ty(&store).maximum.unwrap().0, maximum);
        assert_eq!(run(&instance, &mut store, maximum as i32).unwrap(), 0);
        assert_eq!(memory.size(&store).0, maximum);
        assert_eq!(run(&instance, &mut store, 1).unwrap(), -1);
        assert_eq!(memory.size(&store).0, maximum);
    }
}

#[test]
fn table_growth_obeys_host_cap_and_smaller_declared_maximum() {
    for (limits, maximum) in [("0", 65_536), ("0 100000", 65_536), ("0 2", 2)] {
        let source = format!(
            "(module (table (export \"table\") {limits} funcref)
                (func (export \"run\") (param i32) (result i32)
                    ref.null func local.get 0 table.grow))"
        );
        let (mut store, instance) = instance(&source, 1 << 40);
        let table = instance.exports.get_table("table").unwrap();
        assert_eq!(table.ty(&store).maximum, Some(maximum));
        assert_eq!(run(&instance, &mut store, maximum as i32).unwrap(), 0);
        assert_eq!(table.size(&store), maximum);
        assert_eq!(run(&instance, &mut store, 1).unwrap(), -1);
        assert_eq!(table.size(&store), maximum);
    }
}

struct BulkCase {
    name: &'static str,
    source: String,
    unit_cost: u64,
    result: i32,
}

fn bulk_cases() -> Vec<BulkCase> {
    let memory = "(memory (export \"memory\") 1)";
    let table = "(table (export \"table\") 8192 funcref)";
    let answer = "(type $answer (func (result i32))) (func $answer (type $answer) i32.const 42)";
    let call = "i32.const 0 call_indirect (type $answer)";
    [
        (
            "memory.copy",
            format!("{memory} (data (i32.const 32768) \"*\")"),
            "i32.const 0 i32.const 32768 local.get 0 memory.copy i32.const 0 i32.load8_u".into(),
            1,
            42,
        ),
        (
            "memory.fill",
            memory.into(),
            "i32.const 0 i32.const 42 local.get 0 memory.fill i32.const 0 i32.load8_u".into(),
            1,
            42,
        ),
        (
            "memory.init",
            format!("{memory} (data $bytes \"{}\")", "*".repeat(2048)),
            "i32.const 0 i32.const 0 local.get 0 memory.init $bytes i32.const 0 i32.load8_u".into(),
            1,
            42,
        ),
        (
            "memory.grow",
            "(memory (export \"memory\") 0)".into(),
            "local.get 0 memory.grow".into(),
            65_536,
            0,
        ),
        (
            "table.copy",
            format!("{table} {answer} (elem (i32.const 4096) $answer $answer)"),
            format!("i32.const 0 i32.const 4096 local.get 0 table.copy {call}"),
            16,
            42,
        ),
        (
            "table.fill",
            format!("{table} {answer} (elem declare func $answer)"),
            format!("i32.const 0 ref.func $answer local.get 0 table.fill {call}"),
            16,
            42,
        ),
        (
            "table.init",
            format!(
                "{table} {answer} (elem $entries func {})",
                "$answer ".repeat(2048)
            ),
            format!("i32.const 0 i32.const 0 local.get 0 table.init $entries {call}"),
            16,
            42,
        ),
        (
            "table.grow",
            "(table (export \"table\") 0 funcref)".into(),
            "ref.null func local.get 0 table.grow".into(),
            16,
            0,
        ),
    ]
    .into_iter()
    .map(|(name, declarations, body, unit_cost, result)| BulkCase {
        name,
        source: format!(
            "(module {declarations} (func (export \"run\") (param i32) (result i32) {body}))"
        ),
        unit_cost,
        result,
    })
    .collect()
}

fn assert_destination_untouched(case: &BulkCase, instance: &Instance, store: &mut Store) {
    if case.name.starts_with("memory") {
        let memory = instance.exports.get_memory("memory").unwrap();
        if case.name == "memory.grow" {
            assert_eq!(memory.size(&*store).0, 0, "{}", case.name);
        } else {
            let mut byte = [255];
            memory.view(&*store).read(0, &mut byte).unwrap();
            assert_eq!(byte, [0], "{}", case.name);
        }
    } else {
        let table = instance.exports.get_table("table").unwrap();
        if case.name == "table.grow" {
            assert_eq!(table.size(&*store), 0, "{}", case.name);
        } else {
            assert!(
                matches!(table.get(store, 0), Some(Value::FuncRef(None))),
                "{}",
                case.name
            );
        }
    }
}

#[test]
fn every_bulk_operation_charges_runtime_count_before_changing_resources() {
    for case in bulk_cases() {
        let fuel = if case.name == "memory.grow" {
            100_000
        } else {
            1024
        };
        let (mut store, instance) = instance(&case.source, fuel);
        assert_eq!(
            run(&instance, &mut store, 1).unwrap(),
            case.result,
            "{}",
            case.name
        );
        assert!(remaining(&instance, &mut store) < fuel);

        let (mut store, instance) = self::instance(&case.source, fuel);
        let large = if case.name == "memory.grow" { 2 } else { 2048 };
        assert!(run(&instance, &mut store, large).is_err(), "{}", case.name);
        assert_eq!(
            get_remaining_points(&mut store, &instance),
            MeteringPoints::Exhausted,
            "{}",
            case.name
        );
        assert_destination_untouched(&case, &instance, &mut store);
    }
}

#[test]
fn bulk_fuel_scales_with_bytes_pages_and_table_elements() {
    for case in bulk_cases() {
        let mut consumed = Vec::new();
        for count in [1, 2] {
            let (mut store, instance) = instance(&case.source, 1_000_000);
            assert_eq!(
                run(&instance, &mut store, count).unwrap(),
                case.result,
                "{}",
                case.name
            );
            consumed.push(1_000_000 - remaining(&instance, &mut store));
        }
        // These runs execute identical opcodes. Only the runtime size changes.
        assert_eq!(consumed[1] - consumed[0], case.unit_cost, "{}", case.name);
    }
}

#[test]
fn negative_counts_exhaust_small_budgets_without_wrapping() {
    for case in bulk_cases() {
        for count in [i32::MIN, -1] {
            let (mut store, instance) = instance(&case.source, 1_000_000);
            assert!(
                run(&instance, &mut store, count).is_err(),
                "{} {count}",
                case.name
            );
            assert_eq!(
                get_remaining_points(&mut store, &instance),
                MeteringPoints::Exhausted,
                "{} {count}",
                case.name
            );
            assert_destination_untouched(&case, &instance, &mut store);
        }
    }
}

#[test]
fn max_unsigned_count_has_finite_u64_cost_before_normal_bounds_failure() {
    for case in bulk_cases() {
        let initial = 1 << 60;
        let (mut store, instance) = instance(&case.source, initial);
        let result = run(&instance, &mut store, -1);
        if case.name.ends_with(".grow") {
            assert_eq!(result.unwrap(), -1, "{}", case.name);
        } else {
            assert!(result.is_err(), "{}", case.name);
        }
        let charged = initial - remaining(&instance, &mut store);
        let expected = u64::from(u32::MAX) * case.unit_cost;
        // Instruction charges are tiny compared with the bulk request. A signed
        // extension, i32 multiply, or missing charge cannot meet this bound.
        assert!(
            (expected..expected + 64).contains(&charged),
            "{}: charged {charged}, expected {expected} plus instructions",
            case.name
        );
        assert_destination_untouched(&case, &instance, &mut store);
    }
}

#[test]
fn guests_cannot_address_appended_metering_or_scratch_globals() {
    for source in [
        "(module (func i64.const -1 global.set 0))",
        "(module (func i32.const 0 global.set 1))",
        "(module (func i32.const 0 global.set 2))",
        "(module (global (mut i32) (i32.const 0)) (func i64.const -1 global.set 1))",
    ] {
        let store = store_with_fuel(100);
        assert!(
            matches!(Module::new(&store, source), Err(CompileError::Validate(_))),
            "{source}"
        );
    }
}

#[test]
fn shared_memories_and_atomic_waits_are_rejected_before_execution() {
    for source in [
        "(module (memory 1 1 shared))",
        "(module (memory 1) (func i32.const 0 i32.const 0 i64.const -1 memory.atomic.wait32 drop))",
        "(module (memory 1) (func i32.const 0 i64.const 0 i64.const -1 memory.atomic.wait64 drop))",
    ] {
        let store = store_with_fuel(100);
        assert!(
            matches!(Module::new(&store, source), Err(CompileError::Validate(_))),
            "{source}"
        );
    }
}

#[test]
fn direct_guest_recursion_traps_on_stack_limit_before_fuel_exhaustion() {
    let source = "(module
        (func $recurse (export \"run\") (param i32) (result i32)
            local.get 0 call $recurse
            i32.const 1 i32.add))";
    let initial = 100_000_000;
    let (mut store, instance) = instance(source, initial);
    let error = run(&instance, &mut store, 0).unwrap_err();
    assert_eq!(error.to_trap(), Some(wasmer_types::TrapCode::StackOverflow));
    let fuel = remaining(&instance, &mut store);
    assert!(fuel > 0 && fuel < initial);
}
