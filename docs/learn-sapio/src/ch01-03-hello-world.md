# Hello World

Let's get going with your very first hello world contract!

Unfortunately, until Sapio becomes a little more popular the embedded rust
playground won't work, so you'll want to copy it locally.

We're going to start with a contract that allows two parties, Alice and Bob,
to either agree on an outcome or to default to a pre-fixed outcome after a
relative timeout.

```rust
use bitcoin::util::amount::CoinAmount;
use sapio::contract::*;
use sapio::*;
use sapio_base::timelocks::RelTime;
use sapio_base::Clause;
pub struct TrustlessEscrow {
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    alice_escrow: (CoinAmount, bitcoin::Address),
    bob_escrow: (CoinAmount, bitcoin::Address),
}

impl TrustlessEscrow {
    guard! {
        fn cooperate(self, ctx) {
            Clause::And(vec![Clause::Key(self.alice), Clause::Key(self.bob)])
        }
    }
    then! {
        fn use_escrow(self, ctx) {
            ctx.template()
                .add_output(
                    self.alice_escrow.0.try_into()?,
                    &Compiled::from_address(self.alice_escrow.1.clone(), None),
                    None)?
                .add_output(
                    self.bob_escrow.0.try_into()?,
                    &Compiled::from_address(self.bob_escrow.1.clone(), None),
                    None)?
                .set_sequence(0, RelTime::try_from(std::time::Duration::from_secs(10*24*60*60))?.into())?.into()
        }
    }
}

impl Contract for TrustlessEscrow {
    declare! {finish, Self::cooperate}
    declare! {then, Self::use_escrow}
    declare! {non updatable}
}
```

Create a new rust project and paste the above code in. You should be able to
compile it using `cargo build`. 

## Challenges

1. Add a new finish state that allows Alice to spend after a relative timeout.
1. Add `use_escrow2` which enables a different pair of payouts to Alice and
   Bob as an alternative.
