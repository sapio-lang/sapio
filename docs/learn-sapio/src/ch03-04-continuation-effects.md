# Typed action requests and effects

A contract's spending policy can be compiled without making a request to every
action. Each suggested action publishes its own argument schema and effects path.
The generated typed handle encodes a request at that exact path:

```rust
let effects = Escrow::pay_action().request(&root_path, &payment)?;
let context = Context::new(
    network,
    amount,
    lowering,
    root_path,
    Arc::new(effects),
    None,
);
let artifact = escrow.compile(context)?;
```

The request invokes only `pay`; it is not passed through unrelated actions or a
shared contract-wide enum. JSON deserialization occurs at the request boundary.
`Escrow::pay_action().invoke(&escrow, context, payment)` is the equivalent typed
Rust construction entry point. Directly calling `escrow.pay(context, payment)`
also works because the contract macro preserves ordinary methods.

For several candidates at the same action, use
`Escrow::pay_action().requests(&root_path, &payments)`. Entries receive distinct,
deterministic labels that preserve their input order. An empty collection adds no
requests. The lower-level `MapEffectDB` remains available when requests must be
attached at multiple contract paths.

An argument-free suggested action still needs an explicit unit request, encoded
as JSON `null`. An absent entry never means “call this action with a default
value.” Explicit default-proposal callbacks are evaluated separately.

Suggested transactions remain subject to their fixed authorization policy. A
candidate does not acquire authority because its generator accepted it. For
example, an NFT sale generator may construct the transfer and seller payment;
the owner must still authorize the resulting transaction through the spending
policy. A committed action additionally fixes its candidate transactions in the
compiled covenant, so changing committed candidates may change the address.

Raw effects paths that are not visited by compilation are not automatically
reported as unused. Typed handles avoid hand-written action paths; callers that
assemble raw effects maps remain responsible for targeting the intended contract
instance.
