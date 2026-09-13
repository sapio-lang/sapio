# Compile-time action conditions

`ConditionallyCompileIf` enables a contract writer to evaluate certain
value-based logic before evaluating a path function.

If the return value(s) indicate that a branch should not be evaluated, it is
skipped.

## When to Use ConditionallyCompileIf

Suppose we're creating a super secure wallet vault, and we want a recovery
path that's only accessible if the amount of funds being sent to the contract is < an amount.

We could write:

```rust
#[condition]
fn not_too_much(&self, ctx: Context) -> ConditionalCompileType {
    if ctx.funds() > Self::MAX_FUNDS {
        ConditionalCompileType::Never
    } else {
        ConditionalCompileType::NoConstraint
    }
}
```

Inside `#[sapio::contract]`, apply it with
`#[action(committed, compile_if(Self::not_too_much))]`. This controls whether the
compiler includes the action. It does not introduce a predicate checked while
spending; spending rules belong in policies.

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

## Optional interface conditions

The standalone attribute and declaration macro support optional trait methods:

```rust
#[compile_if]
fn available(self, ctx: Context) -> ConditionalCompileType {
    ConditionalCompileType::NoConstraint
}

// In a trait interface, its factory is absent unless implemented:
decl_compile_if! { available }
```

Both frontends preserve the same condition algebra. `Never` and `Required`
contradict one another; an explicit failure does not hide this contradiction.
An absent condition factory keeps the declared slots of the remaining
conditions, preserving their context paths.
