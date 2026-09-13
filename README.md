# Sapio

Sapio is a Rust framework for describing Bitcoin contracts as graphs of
transactions. Contracts compile into spending conditions, transaction templates,
and metadata; the tooling can bind those templates to UTXOs and produce PSBTs.

This checkout is undergoing modernization. The compiler, signing code and WASM
boundary have regression fixes, and builds use a pinned stable Rust toolchain.
The [modernization plan](docs/MODERNIZATION.md) records what is implemented and
what still blocks a supported release.

## Start here

Install [Rust with rustup](https://www.rust-lang.org/tools/install) and a native C
compiler. From the repository root:

```sh
cargo run --locked -p sapio --example payment
cargo test --locked --workspace --all-features
```

Rustup selects the version in `rust-toolchain.toml`. The
[payment example](sapio/examples/payment.rs) compiles a 1,000-satoshi payment with
500 satoshis reserved for fees and prints the contract as JSON. It needs no node
or signer and does not fund or broadcast a transaction. The integration tests
start their own emulator on a local ephemeral port.

Native CTV compilation is a **research target**. A generated address does not
establish that the target chain enforces CTV. Signer emulation has separate trust
and availability assumptions. See the [enforcement model](docs/MODERNIZATION.md#enforcement-and-release-boundaries)
before using either with funds.

For WASM modules and development checks, follow the
[development guide](docs/DEVELOPMENT.md). The historical
[Designing Bitcoin Contracts with Sapio](docs/learn-sapio/src/SUMMARY.md) book
contains broader examples; its older installation instructions are being revised.

Start with [transaction plans and typed actions](docs/TRANSACTION_PLANS.md),
then [spend planning](docs/SPEND_PLANNING.md) and the
[artifact explainer](cli/README.md#explain-a-contract). These APIs distinguish construction,
spending predicates and the evidence required to satisfy one branch.

The [contract example catalog](docs/EXAMPLES.md) inventories every library family,
all 20 WASM modules and runnable native examples, with regression coverage
and the assumptions each construction still requires.

## Repository map

| Component | Purpose |
| --- | --- |
| [sapio](sapio/) | Contract traits, compiler, transaction templates and linking |
| [sapio-base](sapio-base/) | Bitcoin types, CTV hashing, amounts and shared formats |
| [sapio-psbt](sapio-psbt/) | Taproot PSBT signing |
| [sapio_macros](sapio_macros/) | Rust contract authoring macros |
| [cli](cli/) | Contract, PSBT and emulator commands |
| [plugins](plugins/) | WASM client ABI and host runtime |
| [ctv_emulators](ctv_emulators/) | Signer-based CTV emulation |
| [sapio-contrib](sapio-contrib/) | Contract library and research examples |
| [plugin-example](plugin-example/) | Separately built WASM example workspace |
| [integration_tests](integration_tests/) | Compilation, signing and finalization checks |

Read Jeremy Rubin's [A Calculus of Covenants](https://rubin.io/bitcoin/2022/04/12/calc-cov/)
for the conceptual foundation. Development should preserve the connection between
intended transitions, their verifier, their prover, and the assumptions under
which they agree.

Sapio is licensed under [MPL-2.0](LICENSE). Existing ownership and contribution
terms have not been changed by this modernization.
