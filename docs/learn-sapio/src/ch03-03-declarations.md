# Contract declarations

`#[sapio::contract]` registers explicitly marked actions and spending policies.
Ordinary helper methods remain ordinary helpers. `#[policy]` declares a policy
that can guard actions; `#[spend]` also exports it as independently sufficient to
spend the output.

Contract hooks are ordinary methods marked `#[amount]` for minimum funding,
`#[internal_key]` for an already authorized Taproot internal key, and `#[metadata]`
for descriptive object metadata. Selecting an internal key never grants new
spending authority.

An explicit `Contract` implementation can register factories directly:

```rust
impl Contract for Escrow {
    declare! {actions, Self::refund, Self::propose_payment}
    declare! {finish, Self::cooperative}
}
```

Each action has its own request type before registration. A factory returns
`Option<Box<dyn ErasedAction<Self>>>`; returning `None` explicitly omits an
optional interface member. Action names must be unique within a contract so typed
request paths cannot be silently redirected.

`DynamicContract<S>` assembles `actions` and independent `finish` factories in
vectors alongside the contract data, metadata callback and minimum-funding
callback. A custom `AnyContract` implementation can provide the same compiler
interface without choosing a specific storage layout.

Existing addresses can be used through `Compiled::from_address`. They provide an
output destination and minimum-funding information, but no action API or source
policy beyond the supplied artifact.
