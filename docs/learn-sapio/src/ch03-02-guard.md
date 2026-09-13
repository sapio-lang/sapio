# Guard

A guard is a fixed spending predicate. It can use native Miniscript, a typed
program evaluator, or another supported policy compiler.

Inside `#[sapio::contract]`, a `#[policy]` method declares a reusable predicate.
A `#[spend]` method additionally exports its predicate as independently sufficient
to spend the output. An action attaches policies with
`guarded_by(Self::signed, Self::timeout)`; these predicates are conjoined.

```rust
#[policy]
fn signed(&self) -> Clause {
    Clause::Key(self.owner)
}
```

The ordinary method's signature expresses its context dependency. With only
`&self`, it is cached within the compilation. Adding `Context` evaluates it at
each attachment. The method remains directly callable as Rust.

The standalone `#[guard]` frontend is useful for trait interfaces and optional
metadata callbacks:

An ordinary guard receives the context of each attachment. Use it when the
policy depends on the current path or other compilation context.

A cached guard is evaluated once per guard declaration during a contract's
compilation. It receives only `self`, so it cannot accidentally reuse the first
attachment's context at a different path. Each compilation starts a new cache;
compiling the same contract again evaluates the cached policy again.

## guard macro
```rust
#[guard]
fn contextual(self, ctx: Context) {
    // Compute a Clause using this attachment's context.
}

#[guard(cached)]
fn signed(self) {
    Clause::Key(self.owner)
}
```

These methods belong inside the contract's implementation. Trait interfaces
can declare their corresponding optional methods with
`decl_guard! { contextual }` and `decl_guard! { cached signed }`.

Guard metadata remains contextual even when the policy is cached. A
`simps = "Some(Self::metadata)"` callback receives its own `Context` for every
attachment, including standalone finish guards. Its errors abort compilation.
The compiled artifact groups annotations by guard clause and protocol number,
preserving distinct JSON values in attachment order and removing equal values.
Metadata from different attachment paths therefore remains visible without
duplicating identical annotations.
