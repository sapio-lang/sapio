# Type-level state machines

Rust types can express which actions a contract implementation provides. An
optional action factory returns `None` when a transition is unavailable; this is
separate from an action returning an empty set of transactions.

The following sketch uses `Opened` and `Closed` as state tags. Both states retain
the owner's independent spending policy, while only the open state exports the
committed `close` transition.

```rust
use std::marker::PhantomData;

struct Opened;
struct Closed;
struct StatefulContract<State> {
    owner: bitcoin::XOnlyPublicKey,
    state: PhantomData<State>,
}

trait Moves: Sized + 'static {
    sapio::decl_then! { close }
}

impl Moves for StatefulContract<Opened> {
    #[sapio::then]
    fn close(self, ctx: Context) {
        let amount = ctx.funds();
        ctx.template()
            .add_output(amount, &StatefulContract::<Closed> {
                owner: self.owner,
                state: PhantomData,
            }, None)?
            .into()
    }
}

impl Moves for StatefulContract<Closed> {}

#[sapio::contract(actions(Self::close))]
impl<State: 'static> StatefulContract<State>
where
    Self: Moves,
{
    #[spend]
    fn owner(&self) -> Clause {
        Clause::Key(self.owner)
    }
}
```

`decl_then!` supplies an absent default factory. The open implementation replaces
it; the closed implementation keeps it absent. The contract macro explicitly
exports that optional interface and the independent owner policy.

For availability that depends on a value rather than a Rust type, use a
`#[condition]` method with `compile_if(...)`. For example, an eltoo state can omit
its update action after reaching the maximum state number. `Required` and
`Nullable` still distinguish an action that must generate a transaction from one
that may legitimately return none.

Rust enums, traits, generics and const generics can organize more elaborate state
machines. They execute while constructing and compiling contracts. An ordinary
Rust state check is not automatically enforced by a spending script: the
committed transition or fixed policy/evaluator must enforce the corresponding
on-chain rule.
