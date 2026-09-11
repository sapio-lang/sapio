# Hello World

Let's get going with your very first hello world contract!

Unfortunately, until Sapio becomes a little more popular the embedded rust
playground won't work, so you'll want to copy it locally.

We're going to start with a contract that allows two parties, Alice and Bob,
to either agree on an outcome or to default to a pre-fixed outcome after a
relative timeout.

```rust
//! Hello World Contract

#![deny(missing_docs)]
#[cfg(target_arch = "wasm32")]
use sapio_wasm_plugin::{optional_logo, REGISTER};

use sapio::contract::*;
use sapio::*;
use sapio_base::amount::CoinAmount;
use sapio_base::timelocks::RelTime;
use sapio_base::Clause;
use schemars::JsonSchema;
use serde::Deserialize;
use std::convert::{TryFrom, TryInto};

/// Trustless Escrow Contract
#[derive(JsonSchema, Deserialize)]
pub struct TrustlessEscrow {
    alice: bitcoin::XOnlyPublicKey,
    bob: bitcoin::XOnlyPublicKey,
    alice_escrow_address: bitcoin::Address<bitcoin::address::NetworkUnchecked>,
    alice_escrow_amount: CoinAmount,
    bob_escrow_address: bitcoin::Address<bitcoin::address::NetworkUnchecked>,
    bob_escrow_amount: CoinAmount,
}

impl TrustlessEscrow {
    #[guard]
    fn cooperate(self, _ctx: Context) {
        Clause::And(vec![
            Clause::Key(self.alice).into(),
            Clause::Key(self.bob).into(),
        ])
    }
    #[then]
    fn use_escrow(self, ctx: Context) {
        let network = ctx.network;
        ctx.template()
            .add_output(
                self.alice_escrow_amount.try_into()?,
                &Compiled::from_address(
                    self.alice_escrow_address.clone().require_network(network)?,
                    bitcoin::Amount::ZERO,
                ),
                None,
            )?
            .add_output(
                self.bob_escrow_amount.try_into()?,
                &Compiled::from_address(
                    self.bob_escrow_address.clone().require_network(network)?,
                    bitcoin::Amount::ZERO,
                ),
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

#[cfg(target_arch = "wasm32")]
REGISTER![TrustlessEscrow, "logo.png"];
```

The implementation is in `plugin-example/helloworld/src/plugin.rs`. Its
JSON addresses deserialize as `Address<NetworkUnchecked>` and are checked
against `ctx.network` before becoming outputs. The fixed payout branch uses
Sapio's selected covenant backend; signer emulation includes trust in its
signers.

From the repository root, build this plugin with:

```sh
cargo build --manifest-path plugin-example/Cargo.toml \
  --package sapio-wasm-helloworld --release \
  --target wasm32-unknown-unknown --locked
```

Use the repository's pinned Rust toolchain and an LLVM Clang that supports
`wasm32` for secp256k1's C code. On Linux, set
`export CC_wasm32_unknown_unknown=clang`; on macOS with Homebrew LLVM installed, set
`export CC_wasm32_unknown_unknown="$(brew --prefix llvm)/bin/clang"`.
The default output is
`plugin-example/target/wasm32-unknown-unknown/release/sapio_wasm_helloworld.wasm`
(unless you override Cargo's target directory).

## Challenges

For the challenges, you'll want to modify the helloworld plugin file directly.
Through this tutorial we'll use this as a sandbox file.

1. Add a new finish state that allows Alice to spend after a relative timeout.
1. Add `use_escrow2` which enables a different pair of payouts to Alice and
   Bob as an alternative.
