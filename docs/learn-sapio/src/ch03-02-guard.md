# Guard

Guards are central to any Sapio contract. They allow declaring a piece of
miniscript logic.

These guards can either be used standalone as unlocking conditions or as a
requirement on a `continuation` or `then` function.

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
