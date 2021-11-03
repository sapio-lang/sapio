# FinishOrFunc
A `FinishOrFunc` is a continuation of a contract that *may* terminate when
all the guarded_by conditions on that object are met, but provides logic for some default continuations and logic for new continuations in light of new information.

`FinishOrFunc`s do not use CTV to ensure execution.

## When to use a FinishOrFunc

An example of where a `FinishOrFunc` could be used is a multisig escrow contract, where if n-of-n interested parties agree to move the funds, the funds can move to any transaction. However, perhaps the escrow operators typically emit a payment to a third party and carry the remaining balances to a new escrow. A `FinishOrFunc` can provide convenient logic shared by all participants for generating what that next transaction should look like.

## finish! macro


The `continuation` macro generates a static `fn() -> Option<FinishOrFunc>` method for a given impl.

There are a few variants of how you can create a `continuation`.

```rust
struct UpdateType;
/// Helper
fn default_coerce(
    k: <T as Contract>::StatefulArguments,
) -> Result<UpdateType, CompilationError> {
    Ok(k)
}
/// A Guarded CTV Function
#[continuation(
    /// required: guards for the miniscript clauses required
    guarded_by = "[Self::guard_1,... Self::guard_n]",
    /// optional: Conditional compilation
    compile_if = "[Self::compile_if_1, ... Self::compile_if_n]",
    ///  optional: Enables compiling this for a json callable continuation
    web_api,
    /// helper for coercing args for json api, could be arbitrary
    coerce_args = "default_coerce"
)]
fn name(self, ctx:Context, o:UpdateType) {
    /*Result<Box<Iterator<TransactionTemplate>>>*/
}
/// Null Implementation
decl_finish!(name);
```

The parameter `o` is either called directly or attempted to be coerced from the
higher level `<Self as Contract>::StatefulArguments` which mus enum wrap the
arguments. This means that each `continuation` can have a unique parameter type,
but also be represented as a trait object with a single type. Enums may be used
to pass different arguments to different functions.
