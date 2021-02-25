# Sapio

Welcome!

Sapio is a framework for creating composable multi-transaction Bitcoin Smart Contracts.

### Why is Sapio Different?
Sapio helps you build payment protocol specifiers that oblivious third parties
can participate in being none the wiser.

For example, with Sapio you can generate an address that represents a lightning
channel between you and friend and give that address to a third party service
like an exchange and have them create the channel without requiring any
signature interaction from you or your friend, zero trusted parties, and an
inability to differentiate your address from any other.

That's the tip of the iceberg of what Sapio lets you accomplish.


#### Say more...
Before Sapio, most Bitcoin smart contracts primarily focused on who can redeem
coins when and what unlocking conditions were required (see Ivy,
Policy/Miniscript, etc). A few languages, such as BitML, placed emphasis on
multi-transaction and multi-party use cases.

Sapio in particular focuses on transactions using BIP-119
`OP_CHECKTEMPLATEVERIFY`. `OP_CHECKTEMPLATEVERIFY` enables Bitcoin Script to support
complex multi-step smart contracts without a trusted setup.

Sapio is a tool for defining such smart contracts in an easy way and exporting
easy to integrate APIs for managing open contracts. With Sapio you can turn what
previously would require months or years of careful tinkering with Bitcoin
internals into a 20 minute project and get a fully functional Bitcoin
application.

Sapio has intelligent built in features which help developers design safe smart
contracts and limit risk of losing funds.

For more information on Sapio, check out Jeremy's Reckless VR Talk [Sapio: Stateful Smart Contracts
for Bitcoin with OP_CTV](https://www.youtube.com/watch?v=4vDuttlImPc) and
[slides](https://docs.google.com/presentation/d/1X4AGNXJ5yCeHRrf5sa9DarWfDyEkm6fFUlrcIRQtUw4).

### Show Me The Money! Sapio Crash Course:

#### Installation QuickStart

Clone the project:

```bash
git clone https://github.com/sapio-lang/sapio
```

Install Rust (https://www.rust-lang.org/learn/get-started):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Now you can run:

```bash
cargo run --example server  --features ws
```

This starts a websocket server that can compile and run Sapio contracts! You can connect the server
to [tux](https://github.com/sapio-lang/tux) to run an interactive session.

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
impl<'a> Something {
    /* omitted */
}

/// Something's Contract trait binding
impl<'a> Contract<'a> for Something {
    /// [Optional] declares the unlocking conditions
    declare! {finish, /*omitted*/}
    /// [Optional] declares the CTV next steps
    declare! {then, /*omitted*/}
    /// [Optional] declares the updatable next steps and ArgType
    declare! {updatable<ArgType>, /*omitted*/}
    /// note:
    /// If no updatable, this is explicitly required if not using a nightly
    /// compiler.
    declare! {non updatable}
}
```

Let's look at some examples:


A Basic Pay to Public Key contract can be generated as follows:

```rust
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct PayToPublicKey {
    key: bitcoin::PublicKey
}

impl<'a> PayToPublicKey {
    /// a guard is a definition of a unlocking condition
    guard!(with_key |s| {Clause::Key(s.key)});
}

impl<'a> Contract<'a> for PayToPublicKey {
    /// the unlocking condition must be declared as a finish to be active
    declare! {finish, Self::with_key}
    declare! {non updatable}
}
```

Now let's look at an Escrow Contract. Here either Alice and Escrow, Bob and
Escrow, or Alice and Bob can spend the funds. Clauses are defined via (a patched
version of) [rust-miniscript](https://github.com/rust-bitcoin/rust-miniscript/).

```rust
impl<'a> BasicEscrow {
    guard!(redeem |s| {
            Clause::Threshold(1, vec![
                Clause::Threshold(2, vec![Clause::Key(s.alice), Clause::Key(s.bob)]),
                Clause::And(vec![Clause::Key(s.escrow), Clause::Threshold(1, vec![Clause::Key(s.alice), Clause::Key(s.bob)])])
            ])
        }
    );
}

impl<'a> Contract<'a> for BasicEscrow {
    declare! {finish, Self::redeem}
    declare! {non updatable}
}
```

We can also write this a bit more clearly as:

```rust
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct BasicEscrow2 {
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    escrow: bitcoin::PublicKey
}

impl<'a> BasicEscrow2 {
    guard!(use_escrow |s| {Clause::And(vec![Clause::Key(s.escrow), Clause::Threshold(2, vec![Clause::Key(s.alice), Clause::Key(s.bob)])]) });
    guard!(cooperate |s| {Clause::And(vec![Clause::Key(s.alice), Clause::Key(s.bob)])});
}

impl<'a> Contract<'a> for BasicEscrow2 {
    declare! {finish, Self::use_escrow, Self::cooperate}
    declare! {non updatable}
}
```

Until this point, we haven't made use of any of the `CheckTemplateVerify`
functionality of Sapio. These could all be done in Bitcoin today.

But Sapio lets us go further. What if we wanted to protect from Alice and the
escrow or Bob and the escrow from cheating?


```rust
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct TrustlessEscrow {
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    alice_escrow: (CoinAmount, bitcoin::Address),
    bob_escrow: (CoinAmount, bitcoin::Address)
}

impl<'a> TrustlessEscrow {
    guard!(cooperate |s| {Clause::And(vec![Clause::Key(s.alice), Clause::Key(s.bob)])});
    /// then! macro defines a function to create a list of transactions options
    /// to be required.
    /// The iterator is boxed to permit arbitrary inner types.
    then! {use_escrow |s| {
        let o1 = txn::Output::new(
            s.alice_escrow.0,
            Compiled::from_address(s.alice_escrow.1.clone(), None),
            None,
        )?;
        let o2 = txn::Output::new(
            s.bob_escrow.0,
            Compiled::from_address(s.bob_escrow.1.clone(), None),
            None,
        )?;
        let mut tb = txn::TemplateBuilder::new().add_output(o1).add_output(o2).set_sequence(0, 1700 /*roughly 10 days*/);
        Ok(Box::new(std::iter::once(
            tb.into(),
        )))
    }}
}

impl<'a> Contract<'a> for TrustlessEscrow {
    declare! {finish, Self::cooperate}
    /// we must declare in order for it to be active
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



## Helpful Hints

### Debugging Macros

First, you need to be on the nightly compiler via `rustup default nightly`.

Then, you can run (for example):
```bash
cargo rustc --example=server --features="ws" -- -Zunstable-options --pretty=expanded
```

Which will expand all of the macros in the example "server".
