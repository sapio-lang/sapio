# Suggested actions

A suggested action constructs candidate transactions under a fixed spending
policy. Its authorization guards determine who can spend; returning a candidate
from Rust does not commit the output to that transaction.

For example, an escrow's participants may agree to make a payment and return the
remainder to a new escrow. The action provides shared construction logic, while
the participants' signatures authorize the actual transaction.

```rust
#[sapio::contract]
impl Escrow {
    #[policy]
    fn participants(&self) -> Clause {
        Clause::And(vec![Clause::Key(self.alice).into(), Clause::Key(self.bob).into()])
    }

    #[action(suggested, guarded_by(Self::participants))]
    fn pay(&self, ctx: Context, payment: Payment) -> Result<Template, CompilationError> {
        self.payment_template(ctx, payment)
    }
}
```

`Payment` is this action's own request type. The generated
`Escrow::pay_action()` handle can invoke the method with a typed request or encode
it for compilation through JSON/WASM. There is no contract-wide argument enum.

No request means no invocation. A request-taking action does not need a `Default`
implementation or an artificial `Option` parameter. Even a unit request is an
explicit request: JSON `null` is distinct from an absent effects entry.

When useful, `defaults = Self::default_proposals` attaches a separate callback
that constructs default candidates. It does not manufacture request arguments.
An argument-free suggested action can use the explicit `default` flag instead.

The method's Rust checks validate the generated candidate. Spending requirements
belong in its policy or evaluator; the compiler rejects attempts to add new
authorization guards through a suggested template.

The standalone `#[continuation]` frontend remains available for trait interfaces.
It generates the same `Action<Self, Request>` representation, accepts optional
`defaults = "Self::default_proposals"`, and exposes JSON when marked `web_api`.
