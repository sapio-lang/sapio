# Delegated tree payments

Calls a batching module and funds the returned contract. The selected module must implement a compatible batching wire interface.

See the [workspace guide](../README.md) for Cargo build and test commands,
funding assumptions, and the complete executable catalog. This module has a
[representative input](../../contrib/vectors/examples/trampolinepay.json).

Use the [TreePay batching adapter](../treepay-batching/), whose exported API
exactly matches the batching interface. The general TreePay module has a wider
constructor enum and is not interchangeable with this typed handle.

Coverage: Catalog calls the adapter with native and signer-emulated lowering.
