# Sapio

Sapio's core crate provides Rust contract declarations, transaction constraints,
policy compilation, portable artifacts and PSBT linking. Start with the
[repository README](../README.md) and [development guide](../docs/DEVELOPMENT.md)
for the supported toolchain and project status.

Run the maintained payment example from the repository root:

```sh
cargo run --locked -p sapio --example payment
```

It compiles a contract without funding or broadcasting it. Native CTV remains a
research target; see the [enforcement model](../docs/MODERNIZATION.md#enforcement-and-release-boundaries).

## Ordinary Rust methods, explicit contract semantics

A contract is data plus ordinary Rust methods. `#[sapio::contract]` exports only
explicitly marked actions and spending policies. A simple signature policy needs
no transaction generator or contract-wide argument type:

```rust
use sapio_base::Clause;

struct Signature {
    owner: bitcoin::XOnlyPublicKey,
}

#[sapio::contract]
impl Signature {
    #[spend]
    fn signed(&self) -> Clause {
        Clause::Key(self.owner)
    }
}
```

`#[policy]` declares a reusable policy without making it independently sufficient
to spend. `#[spend]` additionally exports that authority. Policies with no Context
parameter are cached within the compilation; contextual policies are evaluated
at their attachment context.

A committed action fixes its generated transactions under the chosen covenant
lowering. This is the authoring shape used by the
[complete payment example](examples/payment.rs):

```rust
#[sapio::contract]
impl Payment {
    #[action(committed)]
    fn pay(&self, ctx: Context) -> Result<Template, CompilationError> {
        let mut plan = ctx.template_plan();
        plan.output("recipient", OutputAmount::Exact(Amount::from_sat(1_000)), &self.destination)?;
        plan.reserve_fees(Amount::from_sat(500));
        Ok(plan.finish()?)
    }
}
```

The transaction plan names its allocations and resolves them before compiling
child outputs. A normal action may return one template, an explicit vector of
alternatives, or a fallible template iterator.

## Requests and proposals

`#[action(suggested, guarded_by(Self::authorization))]` generates proposals under
a fixed policy. Its method has its own typed request argument; there is no shared
request enum and no fabricated default invocation.

```rust
let effects = Channel::update_action().request(&root_path, &next_state)?;
let proposals = Channel::update_action().invoke(&channel, context, next_state)?;
```

The first expression encodes the request at the exact action path for compilation
or WASM execution. The second directly constructs proposals with a typed Rust
argument. The original `channel.update(...)` method also remains callable.
`requests(&root_path, &candidates)` adds several ordered requests without replacing
one with another.

Default proposals are a separate explicit callback. Argument-free committed
actions generate their fixed templates during compilation; suggested actions
require an explicit request or default callback, including when the request type
is unit.

Rust checks inside a proposal generator do not automatically become spending
predicates. The fixed policy or evaluator must enforce the intended rule.

## Interfaces and further reading

The low-level `Action<Contract, Request>` API is available without macros.
`declare! {actions, ...}` exports individually erased factories; a factory returning
`None` explicitly omits an optional interface action. The standalone `then`,
`continuation`, and `guard` attributes implement the same compiler representation.

See the [language guide](../docs/learn-sapio/src/ch03-03-declarations.md),
[policy backend guide](../docs/POLICY_BACKENDS.md), and tested contracts in
[`sapio-contrib`](../sapio-contrib/src/contracts).
