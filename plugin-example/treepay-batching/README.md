# TreePay batching adapter

Implements `batching_trait::BatchingModule` by exposing exactly
`batching_trait::Versions` and returning a compiled contract. Use this module
for TrampolinePay's batching handle. The general [TreePay module](../treepay/)
has additional constructor variants and therefore a different typed API.

Both entry points reuse [treepay-contract](../treepay-contract/). The adapter
selects radix four, estimates each transaction's fee as 215 bytes multiplied
by the supplied satoshi-per-byte fee rate, and adds no relative delay.
The estimate is the example's funding policy, not a measured transaction fee.
Payments must be nonempty and positive; fee and payment overflow is rejected.

The [catalog input](../../contrib/vectors/examples/treepay-batching.json)
compiles two payments with 215 satoshis in fees. The catalog also invokes this
adapter through TrampolinePay under native and signer-emulated lowering.
