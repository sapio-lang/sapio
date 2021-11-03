# Guard

Guards are central to any Sapio contract. They allow declaring a piece of
miniscript logic.

These guards can either be used standalone as unlocking conditions or as a
requirement on a `continuation` or `then` function.

If a `guard` is marked as cached, the compiler will make an effort to only
invoke the `guard` once during compilation. This is helpful in contexts
where a `guard` might be expensive to call, e.g. if it is programmed to
retrieve a `Clause` from a remote server. It is not guaranteed that the
`guard` is only invoked once.

## guard macro
```rust
#[guard(
    /// optional: if the compiler should attempt to only call this guard one time (not guaranteed)
    cached)]
fn name(self, ctx: Context) {/*Clause*/}
decl_guard!{name};
```
