# Vault with recovery

Provides hot/cold spending, delayed redemption and optional CPFP outputs. The fee rate is sats per 1000 weight units, using an unsigned-size estimate that excludes witness growth; fees round up.

See the [workspace guide](../README.md) for Cargo build and test commands,
funding assumptions, and the complete executable catalog. This module has a
[representative input](../../contrib/vectors/examples/jamesob-vault.json).

Coverage: Native overflow, rounding and compiled-funding tests.
