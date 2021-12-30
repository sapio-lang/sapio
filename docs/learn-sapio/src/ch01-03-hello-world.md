# Hello World

Let's get going with your very first hello world contract!

Unfortunately, until Sapio becomes a little more popular the embedded rust
playground won't work, so you'll want to copy it locally.

We're going to start with a contract that allows two parties, Alice and Bob,
to either agree on an outcome or to default to a pre-fixed outcome after a
relative timeout.

```rust
#[derive(JsonSchema, Deserialize)]
pub struct TrustlessEscrow {
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    alice_escrow_address: bitcoin::Address,
    alice_escrow_amount: CoinAmount,
    bob_escrow_address: bitcoin::Address,
    bob_escrow_amount: CoinAmount,
}

impl TrustlessEscrow {
    #[guard]
    fn cooperate(self, _ctx: Context) {
        Clause::And(vec![Clause::Key(self.alice), Clause::Key(self.bob)])
    }
    #[then]
    fn use_escrow(self, ctx: Context) {
        ctx.template()
            .add_output(
                self.alice_escrow_amount.try_into()?,
                &Compiled::from_address(self.alice_escrow_address.clone(), None),
                None,
            )?
            .add_output(
                self.bob_escrow_amount.try_into()?,
                &Compiled::from_address(self.bob_escrow_address.clone(), None),
                None,
            )?
            .set_sequence(
                0,
                RelTime::try_from(std::time::Duration::from_secs(10 * 24 * 60 * 60))?.into(),
            )?
            .into()
    }
}

impl Contract for TrustlessEscrow {
    declare! {finish, Self::cooperate}
    declare! {then, Self::use_escrow}
    declare! {non updatable}
}

REGISTER![TrustlessEscrow, "logo.png"];

```

Navigate to `sapio/plugin-example/helloworld/plugin.rs` in your code editor.
You'll find this code there. You should be able to compile it using 
`cargo build --target wasm32-unknown-unknown`. 

## Challenges

For the challenges, you'll want to modify the helloworld plugin file directly.
Through this tutorial we'll use this as a sandbox file.

1. Add a new finish state that allows Alice to spend after a relative timeout.
1. Add `use_escrow2` which enables a different pair of payouts to Alice and
   Bob as an alternative.
