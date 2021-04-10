# Guard

Guards are central to any Sapio contract. The allow declaring a piece of
miniscript logic.

These guards can either be used standalone as unlocking conditions or as a
requirement on a `finish!` or `then!` function.

If a `guard!` is marked as cached, the compiler will make an effort to only
invoke the `guard!` once during compilation. This is helpful in contexts
where a `guard!` might be expensive to call, e.g. if it is programmed to
retrieve a `Clause` from a remote server. It is not guaranteed that the
`guard!` is only invoked once.

## guard! macro
```rust
guard!{
    fn name(self, ctx) {/*Clause*/}
}
/// The guard should only be invoked once by the compiler, and the result stored
guard!{
    cached fn name(self, ctx) {/*Clause*/}
}
```
