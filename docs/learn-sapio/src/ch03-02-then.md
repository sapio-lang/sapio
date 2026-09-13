# Committed actions

A committed action constructs the transactions permitted by a covenant. Each
returned transaction is combined with the action's authorization guards and the
configured covenant lowering. This is useful for fixed payouts, timeout paths,
and recursively constructed transaction trees.

```rust
#[sapio::contract]
impl Escrow {
    #[action(committed)]
    fn refund(&self, ctx: Context) -> Result<Template, CompilationError> {
        self.refund_template(ctx)
    }
}
```

An argument-free committed action produces its default transaction during
compilation. A request-taking committed action requires supplied requests or an
explicit default-proposal callback. Its requests participate in compilation of
the output's fixed spending policy: changing them may change the contract's
address.

`guarded_by(Self::authorization)` attaches fixed policies.
`compile_if(Self::availability)` applies a separately declared `#[condition]`
method returning `ConditionalCompileType`. The normal required action must
produce at least one template; `Nullable` permits an empty result and `Never`
omits the action.

A method can return a `Template` or `Result<Template, CompilationError>`. Multiple
alternatives are explicit through `Vec<Template>` or
`Result<Vec<Template>, CompilationError>`; `TxTmplIt` remains available for a
fallible stream.

The standalone `#[then]` frontend remains useful when implementing an optional
trait action declared with `decl_then!`. It creates the same committed action
representation, with an argument-free default callback. Export it through
`declare! {actions, Self::refund}`.
