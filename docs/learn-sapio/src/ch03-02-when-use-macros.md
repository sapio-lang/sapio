# When to use macros

Use `#[sapio::contract]` on an inherent impl for normal contract authoring. Mark
transaction methods with `#[action(committed)]` or `#[action(suggested)]`, policies
with `#[policy]`, and independently sufficient spending policies with `#[spend]`.
The methods remain ordinary Rust methods. The macro generates typed action
handles, request schemas and registration.

A policy with only `&self` is context-free and cached. A policy that also accepts
`Context` is evaluated at its attachment context. The signature expresses the
actual dependency.

The explicit low-level API is `Action<Contract, Request>`. Its constructor takes a
transaction-generation callback. `with_guards`, `with_conditions`, `with_json`,
and `with_defaults` add the corresponding capabilities; `erase()` hides only the
individual request type when registering the action. There is no global argument
pack. This API is useful for dynamically assembled contracts.

Optional interfaces can still use `decl_then!`, `decl_continuation!`, and
`decl_guard!`, implemented by the standalone action/guard attributes. A factory
returning `None` means that the implementation does not provide that interface
member. Export optional factories explicitly through `declare! {actions, ...}` or
`#[sapio::contract(actions(Self::optional), spends(Self::optional_guard))]`.

Use `#[condition]` with `compile_if(...)` for availability depending on the
contract's value. Its condition algebra distinguishes absent, nullable and
required branches; it does not add a spending predicate.
