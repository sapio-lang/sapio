# ThenFunc
A `ThenFunc` is a continuation of a contract that can proceed when all the
guarded_by conditions on that object are met. The `ThenFunc` provides an
iterator of possible next transactions, using CTV to ensure execution.

## When to use a ThenFunc

We've already seen an example of a `then!` function in the wild in [Chapter
1](./ch01-03-hello-world.md). In that example we are guaranteeing that after
a timeout, a specific "return policy" is honored out of the escrow. Unless
Alice and Bob agree to something else, the funds can only be returned via
that transaction.

In general, any time you want a state transition to be "locked in" you should use a `then!`.


## then! macro


The `then!` macro generates a static `fn() -> Option<ThenFunc>` method for a given impl.

There are a few variants of how you can create a `then!`.

```rust
/// A Guarded CTV Function
then!{
    guarded_by: [guard_1, ... guard_n]
    fn name(self, ctx) {
        /*Result<Box<Iterator<TransactionTemplate>>>*/
    }
}
/// A Conditional CTV Function
then!{
    compile_if: [compile_if_1, ... compile_if_n]
    fn name(self, ctx) {
        /*Result<Box<Iterator<TransactionTemplate>>>*/
    }
}
/// A Conditional + Guarded CTV Function
then!{
    compile_if: [compile_if_1, ... compile_if_n]
    guarded_by: [guard_1, ... guard_n]
    fn name(self, ctx) {
        /*Result<Box<Iterator<TransactionTemplate>>>*/
    }
}
/// An Unguarded CTV Function
then!{
    fn name(self, ctx) {
        /*Result<Box<Iterator<TransactionTemplate>>>*/
    }
}
/// Null Implementation
then!{name}
```

The Iterator must not be empty, or it will cause an error.