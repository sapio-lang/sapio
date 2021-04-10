# ConditionallyCompileIf

`ConditionallyCompileIf` enables a contract writer to evaluate certain
value-based logic before evaluating a path function.

If the return value(s) indicate that a branch should not be evaluated, it is
skipped.

## When to Use ConditionallyCompileIf

Suppose we're creating a super secure wallet vault, and we want a recovery
path that's only accessible if the amount of funds being sent to the contract is < an amount.

We could write:

```rust
compile_if!{
    fn not_too_much(self, ctx) {
        if ctx.funds() > Self::MAX_FUNDS {
            ConditionallyCompileType::Never
        } else {
            ConditionalCompileType::NoConstraint
        }
    }
}
```

and apply it to the relevent paths.

## ConditionalCompileType Variants

There are many different ConditionalCompileType return values:

```rust
pub enum ConditionalCompileType {
    /// May proceed without calling this function at all
    Skippable,
    /// If no errors are returned, and no txtmpls are returned,
    /// it is not an error and the branch is pruned.
    Nullable,
    /// The default condition if no ConditionallyCompileIf function is set, the
    /// branch is present and it is required.
    Required,
    /// This branch must never be used
    Never,
    /// No Constraint, nothing is changed by this rule
    NoConstraint,
    /// The branch should always trigger an error, with some reasons
    Fail(LinkedList<String>),
}
```

These values are merged according to specific "common sense" logic. Please
see `ConditionalCompileType::merge` for details.

```rust

    ///     Fail > non-Fail ==> Fail
    ///     forall X. X > NoConstraint ==> X
    ///     Required > {Skippable, Nullable} ==> Required
    ///     Skippable > Nullable ==> Skippable
    ///     Never >< Required ==> Fail
    ///     Never > {Skippable, Nullable}  ==> Never
```

# compile_if! macro
The `compile_if` macro can be called two ways:
```rust
compile_if!{
    fn name(self, ctx) {
        /*ConditionalCompileType*/
    }
}
/// null implementation
compile_if!{name}
```
