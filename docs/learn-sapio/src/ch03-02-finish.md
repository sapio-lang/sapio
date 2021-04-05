# FinishOrFunc
A `FinishOrFunc` is a continuation of a contract that *may* terminate when
all the guarded_by conditions on that object are met, but provides logic for some default continuations and logic for new continuations in light of new information.

`FinishOrFunc`s do not use CTV to ensure execution.

## When to use a FinishOrFunc

An example of where a `FinishOrFunc` could be used is a multisig escrow contract, where if n-of-n interested parties agree to move the funds, the funds can move to any transaction. However, perhaps the escrow operators typically emit a payment to a third party and carry the remaining balances to a new escrow. A `FinishOrFunc` can provide convenient logic shared by all participants for generating what that next transaction should look like.

## finish! macro


The `finish!` macro generates a static `fn() -> Option<FinishOrFunc>` method for a given impl.

There are a few variants of how you can create a `finish!`.

```rust
/// A Guarded CTV Function
finish!{
    guarded_by: [guard_1, ... guard_n]
    fn name(self, ctx, o) {
        /*Result<Box<Iterator<TransactionTemplate>>>*/
    }
}
/// A Conditional CTV Function
finish!{
    compile_if: [compile_if_1, ... compile_if_n]
    guarded_by: [guard_1, ... guard_n]
    fn name(self, ctx, o) {
        /*Result<Box<Iterator<TransactionTemplate>>>*/
    }
}
/// Null Implementation
finish!(name);
```

The type of the parameter `o` is `Option<<Self as Contract>::StatefulArguments>` and is the same across all `FinishOrFunc`s. Enums may be used to pass different arguments to different functions.
