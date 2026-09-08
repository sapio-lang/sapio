# Signed payment pool

Authenticates payment requests by sequence, fees, payouts and sender; records fees and gives a final member a direct exit. Balance totals must equal the available funds. Keep sig_needed true outside debugging.

See the [workspace guide](../README.md) for Cargo build and test commands,
funding assumptions, and the complete executable catalog. This module has a
[representative input](../../contrib/vectors/examples/payment_pool.json).

Coverage: Native signature mutation, overspending, ejection, fee and withdrawal tests.
