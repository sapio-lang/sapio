# Contract Declarations

## Static Contracts

This is the usual way to declare a contract for Sapio.

Once a contract and all relevant logic has been defined, a `impl Contract`
should be written. This binds the functionality to the compiler interface.

```rust
impl Contract for T {
    declare!{then, Self::a, Self::b}
    declare!{finish, Self::guard_1, Self::guard_2}
    /// if there are finish! functions
    declare!{updatable<Z>, Self::updatable_1}
    /// if there are no updatable functions
    declare!{non updatable}
}
```

The type `Z` above becomes bound for the updatable functions.

## Dynamic Contracts

Sapio also supports several "Dynamic Contract" paradigms which allows a user
to assemble contracts at run-time. The two main paradigms are accomplished by
either directly `impl AnyContract` or by using the `DynamicContract` struct
which holds all functions in vecs.

These are useful in rare circumstances.

## External Addresses?

The compiler is able to "lift" an address or a script into a contract via
`Object::from_address` and `Object::from_script`. Care should be taken when
doing so as Sapio will not be able to provide any further API data beyond such a bound.


