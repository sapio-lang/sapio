# Delegated tree payments

Calls a batching module and funds the returned contract. The selected module must implement a compatible batching wire interface.

See the [workspace guide](../README.md) for Cargo build and test commands,
funding assumptions, and the complete executable catalog. This module has a
[representative input](../../contrib/vectors/examples/trampolinepay.json).

Coverage: Catalog calls the actual treepay guest.
