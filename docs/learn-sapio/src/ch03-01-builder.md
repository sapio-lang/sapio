# Template Builder

The template builder defines a transaction step. Each output receives part of
the available funding and its own compilation context. Reserve fees explicitly
after adding all outputs.

This contract pays 1,000 sats, returns the remaining funds as change, and reserves
100 sats for fees:

```rust
use bitcoin::{Amount, XOnlyPublicKey};
use sapio::contract::{CompilationError, Contract};
use sapio::{declare, then, Context};

struct Payment {
    recipient: XOnlyPublicKey,
    change: XOnlyPublicKey,
}

impl Payment {
    #[then]
    fn pay(self, ctx: Context) {
        let fee = Amount::from_sat(100);
        let mut tmpl = ctx
            .template()
            .set_label("Payment".into())
            .add_output(Amount::from_sat(1_000), &self.recipient, None)?;

        let change = tmpl
            .ctx()
            .funds()
            .checked_sub(fee)
            .ok_or(CompilationError::OutOfFunds)?;
        if change != Amount::ZERO {
            tmpl = tmpl.add_output(change, &self.change, None)?;
        }

        tmpl.add_fees(fee)?.into()
    }
}

impl Contract for Payment {
    declare! {then, Self::pay}
    declare! {non updatable}
}
```

Builder methods consume the previous builder and return the updated one.
`ctx().funds()` reports the remaining construction budget. Emitted outputs and
explicitly reserved fees determine the template's minimum funding requirement;
an unused budget is not a fee reservation.

`add_output` passes the output's amount to the receiving contract. When ordinal
ranges are known, outputs receive consecutive prefixes in transaction input
order. Debiting the builder without creating an output would shift those
ordinal assignments, so direct `spend_amount` access is private.

`add_fees` records the fee and changes the builder's state. That state allows
additional fee reservations within the remaining budget, but cannot add more
outputs or auxiliary funds. This keeps fees after all outputs. Complete
metadata, guards, input sequences and change outputs before reserving fees.

Auxiliary input contributions use `add_sequence().add_amount(amount)?`; these
funds are separate from the contract input's required contribution. When input
ordinals are tracked, allocate all known sats to outputs before introducing
unknown auxiliary funds. Binding checks the actual funding inputs.

Sapio currently places the contract's UTXO at input zero. The CTV commitment
includes this index. See the [builder implementation](https://github.com/sapio-lang/sapio/blob/master/sapio/src/template/builder.rs)
for the complete set of operations.
