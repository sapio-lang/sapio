# Transaction plans

Start with a plan when a contract needs exact payments, change, a fee budget or
named sponsor inputs. The plan resolves allocations before compiling children,
then freezes the exact ordered transaction into the compiler's usual template.

The runnable payment example uses ordinary Rust methods under `#[sapio::contract]`.
Its committed action declares one recipient and reserves a fee. Try it from the
repository root:

```sh
cargo run --locked -p sapio --example payment
```

The complete source below is included directly from the tested example:

```rust,ignore
{{#include ../../../sapio/examples/payment.rs}}
```

`OutputAmount::Remainder` can assign change to one additional output. Output order
is declaration order; the resolver never sorts destinations. Auxiliary inputs
have names and declared minimum contributions, checked again when funding is
supplied. Repeated fee reservations take their maximum, and height/time lock
conflicts are explicit errors.

A fee cap is a local preparation rule. It is not automatically enforced by
Bitcoin Script. Final fee-rate checks also need actual previous outputs and the
completed witness weight. Native signature finalization and chain checks remain
separate from transaction construction.

The lower-level builder described next is useful for procedural constructions.
Both APIs produce the same `Template`.
