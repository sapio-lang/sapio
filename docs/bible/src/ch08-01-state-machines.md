# Type Level State Machines

In this example we use type level state machines to encode functionality that
is potentially available. See the example below for a sketch of how this can work.


```rust
/// The contract we're building, that can be in any type-state T.
struct StatefulContract<T>(PhantomData<T>);

/// We use empty structs as type tags.
/// Note: we could add a `trait State`, but it is not required
/// 
/// A contract can be in the open state or the closed state.
struct Opened;
struct Closed;

/// The "state machine" defines functionality that may be available
trait FunctionalityAtState 
where Self : Sized + Contract
{
    /// empty declaration *could* be a default implementation, but we leave it empty
    /// so that other states may override it.
    then!{do_something}
}


/// Override the impl when state is Opened
impl FunctionalityAtState for StatefulContract<Opened> {
    then! {
        /// Transition from Opened => Closed state
        fn do_something(self, ctx) {
            ctx.template()
               .add_output(ctx.funds(),
                           &StatefulContract::<Closed>(Default::default()),
                           None)?.into()
        }
    }
}

/// do not override `do_something`, no branch will be generated
impl FunctionalityAtState for StatefulContract<Closed> {}

/// Register that all StatefulContract<T>'s that implement FunctionalityAtState
/// are Contracts
impl Contract for StatefulContract<T>
where Self : FunctionalityAtState {
    declare!{then, Self::do_something}
}
```

This technique is *ridiculously* powerful. Imagine, for instance, that we
wanted to have different sorts of state other than Open and Closed. E.g., Red
and Green. We could then define Transition Rules that encode a graph like:

```
(Open, Green) ==> do_something ==> (Closed, Green)
(Open, Red) ==> do_something ==> (Closed, Red)
(Open, Green) ==> do_something_else ==>  (Open, Red)
(Open, Red) ==> do_something_else ==> (Closed, Red)
```
using two separate `FunctionalityAtState` like traits:

```rust
/// The contract we're building, that can be in any type-state T.
struct StatefulContract<T1, T2>(PhantomData<(T1, T2)>);

/// We use empty structs as type tags.
/// Note: we could add a `trait State`, but it is not required
/// 
/// A contract can be in the open state or the closed state.
struct Opened;
struct Closed;
// And Red or Green
struct Red;
struct Green;

/// The "state machine" defines functionality that may be available
trait OpenAtState 
where Self : Sized + Contract
{
    /// empty declaration *could* be a default implementation, but we leave it empty
    /// so that other states may override it.
    then!{do_something}
}

trait ColorAtState 
where Self : Sized + Contract
{
    /// empty declaration *could* be a default implementation, but we leave it empty
    /// so that other states may override it.
    then!{do_something}
}


/// Override the impl when state is Opened
impl OpenAtState<DontCare> for StatefulContract<Opened, DontCare> {
    then! {
        /// Transition from Opened => Closed state
        fn do_something(self, ctx) {
            ctx.template()
               .add_output(ctx.funds(),
                           &StatefulContract::<Closed, DontCare>(Default::default()),
                           None)?.into()
        }
    }
}

/// do not override `do_something`, no branch will be generated
impl OpenAtState<DontCare> for StatefulContract<Closed, DontCare> {}

/// Override the impl when state is Opened
impl ColorAtState for StatefulContract<Open, Green> {
    then! {
        /// Transition from Green => Red state
        fn do_something_else(self, ctx) {
            ctx.template()
               .add_output(ctx.funds(),
                           &StatefulContract::<Open, Red>(Default::default()),
                           None)?.into()
        }
    }
}

impl ColorAtState for StatefulContract<Open, Red> {
    then! {
        /// Transition from Open => Closed state
        fn do_something_else(self, ctx) {
            ctx.template()
               .add_output(ctx.funds(),
                           &StatefulContract::<Closed, Red>(Default::default()),
                           None)?.into()
        }
    }
}

/// do not override `do_something_else`, no branch will be generated
impl ColorAtState<DontCare> for StatefulContract<DontCare, Red> {}

/// Register that all StatefulContract<T>'s that implement OpenAtState
/// are Contracts
impl Contract for StatefulContract<T>
where Self : OpenAtState + ColorAtState {
    declare!{then, Self::do_something, Self::do_something_else}
}
```


This technique showcases how Sapio could encode very sophisticated logic in
program generation.

It's also notable that following rustc v1.51, it is possible to use `const`'s
as generic type parameters which enables even more computation at the type level.