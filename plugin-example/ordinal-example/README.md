# Ordinal sale

Preserves the requested sat at the start of a 501-sat buyer output. The direct
sale uses `TemplatePlan` with an explicit `require_ordinal` constraint, named
allocations, separate buyer funding and a strict fee cap. The alternative sale
uses the ordinal allocator to choose a layout. Ordered input ranges must be
complete, non-overlapping and include the target plus padding.

See the [workspace guide](../README.md) for Cargo build and test commands,
funding assumptions, and the complete executable catalog. This module has a
[representative input](../../contrib/vectors/examples/ordinal-example.json).

Coverage: Native target-position, payment, fee, invalid-range and planner tests;
a real WASM sale request checks ordered payouts, retained funding constraints
and complete native/WASM artifact equality.
