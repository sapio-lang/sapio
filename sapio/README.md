# Sapio

Sapio's core crate provides Rust contract traits, authoring macros, transaction
compilation and PSBT linking. For the current setup and project status, start
with the [repository README](../README.md) and
[development guide](../docs/DEVELOPMENT.md).

From the repository root:

```sh
cargo run --locked -p sapio --example payment
```

The checked-in [payment example](examples/payment.rs) is the maintained starting
point. It compiles a contract without funding or broadcasting it. Native CTV is
a research target; see the [enforcement model](../docs/MODERNIZATION.md#enforcement-and-release-boundaries).

The sketches below explain the original authoring model and are historical
reference material. They are not a substitute for the tested example.

#### Learning Sapio


Let's look at some example Sapio contracts (see
[the example contracts](https://github.com/JeremyRubin/sapio/tree/master/sapio-contrib/src/contracts) for more
examples).

All contracts have 3 basic parts: a struct definition, some set of methods, and a Contract trait
impl.

```rust
/// deriving these on Something let it interface with external
/// interfaces easily
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct Something {
    /* omitted */
}

/// Something's methods. Note 'a required for macros
impl Something {
    /* omitted */
}

/// Something's Contract trait binding
impl Contract for Something {
    /// [Optional] declares the unlocking conditions
    declare! {finish, /*omitted*/}
    /// [Optional] declares the CTV next steps
    declare! {then, /*omitted*/}
    /// [Optional] declares the updatable next steps and ArgType
    declare! {updatable<ArgType>, /*omitted*/}
    /// Use this instead of updatable<ArgType> when there are no continuations:
    // declare! {non updatable}
}
```

Let's look at some examples:


A Basic Pay to Public Key contract can be generated as follows:

```rust
/// Pay To Public Key Sapio Contract
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct PayToPublicKey {
    key: bitcoin::PublicKey,
}

impl PayToPublicKey {
    guard! {fn with_key(self, ctx) { Clause::Key(self.key) }}
}

impl Contract for PayToPublicKey {
    declare! {finish, Self::with_key}
    declare! {non updatable}
}
```

Now let's look at an Escrow Contract. Here either Alice and Escrow, Bob and
Escrow, or Alice and Bob can spend the funds. Clauses are defined via (a patched
version of) [rust-miniscript](https://github.com/rust-bitcoin/rust-miniscript/).

```rust
/// Basic Escrowing Contract
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct BasicEscrow {
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    escrow: bitcoin::PublicKey,
}

impl BasicEscrow {
    guard! {
        fn redeem(self, ctx) {
            Clause::Threshold(
                1,
                vec![
                    Clause::Threshold(2, vec![Clause::Key(self.alice), Clause::Key(self.bob)]),
                    Clause::And(vec![
                        Clause::Key(self.escrow),
                        Clause::Threshold(1, vec![Clause::Key(self.alice), Clause::Key(self.bob)]),
                    ]),
                ],
            )
        }
    }
}

impl Contract for BasicEscrow {
    declare! {finish, Self::redeem}
    declare! {non updatable}
}
```

We can also write this a bit more clearly as:

```rust

/// Basic Escrowing Contract, written more expressively
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct BasicEscrow2 {
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    escrow: bitcoin::PublicKey,
}

impl BasicEscrow2 {
    guard! {
        fn use_escrow(self, ctx) {
            Clause::And(vec![
                Clause::Key(self.escrow),
                Clause::Threshold(2, vec![Clause::Key(self.alice), Clause::Key(self.bob)]),
            ])
        }
    }
    guard! {
        fn cooperate(self, ctx) { Clause::And(vec![Clause::Key(self.alice), Clause::Key(self.bob)]) }
    }
}

impl Contract for BasicEscrow2 {
    declare! {finish, Self::use_escrow, Self::cooperate}
    declare! {non updatable}
}
```

Until this point, we haven't made use of any of the `CheckTemplateVerify`
functionality of Sapio. These could all be done in Bitcoin today.

But Sapio lets us go further. What if we wanted to protect from Alice and the
escrow or Bob and the escrow from cheating?


```rust
/// Trustless Escrowing Contract
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct TrustlessEscrow {
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    alice_escrow: (CoinAmount, bitcoin::Address),
    bob_escrow: (CoinAmount, bitcoin::Address),
}

impl TrustlessEscrow {
    guard! {
    fn cooperate (self, ctx ) { Clause::And(vec![Clause::Key(self.alice), Clause::Key(self.bob)]) }
    }
    then! {fn use_escrow(self, ctx) {
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
    }}
}

impl Contract for TrustlessEscrow {
    declare! {finish, Self::cooperate}
    declare! {then, Self::use_escrow}
    declare! {non updatable}
}
```


Now with `TrustlessEscrow`, we've done a few things differently. A `then!`
designator tells the contract compiler to add a branch which *must* create the
returned transaction if that branch is taken. We've also passed in a
sub-contract for both Alice and Bob to allow us to specify at a higher layer
what kind of pay out they receive. Lastly, we used a call to `set_sequence` to
specify that we should have to wait 10 days before using the escrow (we could
pass this as a parameter if we wanted though).

Sapio will look to make sure that all paths of our contract are sufficiently
funded, only losing an amount for fees (user configurable).



## Further reading

The [modernization plan](../docs/MODERNIZATION.md) describes the intended compiler
graph, artifact and backend boundaries. The
[historical book](../docs/learn-sapio/src/SUMMARY.md) covers more constructions.
