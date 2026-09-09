# Tree payments

Builds a bounded-radix tree with a fixed fee per transaction. Recipients and amounts must be nonempty and positive; radix must be at least two.

See the [workspace guide](../README.md) for Cargo build and test commands,
funding assumptions, and the complete executable catalog. This module has a
[representative input](../../contrib/vectors/examples/treepay.json).

Coverage: Tree shape, payment totals, fees, invalid radix and overflow.
