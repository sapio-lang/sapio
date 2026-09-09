# Inscription reveal

Commits an Ord envelope under the owner signature and reveals to the owner or a selected address. Complete non-overlapping ranges and enough funds for the inscribed sat, padding and fee are required.

See the [workspace guide](../README.md) for Cargo build and test commands,
funding assumptions, and the complete executable catalog. This module has a
[representative input](../../contrib/vectors/examples/ordinal-inscription.json).

Coverage: Native signed PSBT/artifact round trip, body chunking and invalid-input tests; CLI smoke.
