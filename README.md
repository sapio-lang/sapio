# Sapio

Sapio is a Rust framework for describing Bitcoin contracts as graphs of
transactions. Contracts compile into spending conditions, transaction templates,
and metadata; the tooling can bind those templates to UTXOs and produce PSBTs.

The developer preview includes typed contract actions, transaction plans,
artifact inspection and selected-spend completion. Builds use a pinned stable
Rust toolchain. The [release-readiness record](docs/RELAUNCH.md) distinguishes
the maintained developer path from the remaining deployment and release gates.

## Start here

Follow the [quickstart](docs/QUICKSTART.md) to generate a complete Rust project
and carry one payment from source through a verified synthetic spend. Install
Rust with rustup and a native C compiler, then from the repository root:

```sh
cargo build --locked -p sapio-cli
export PATH="$PWD/target/debug:$PATH"
sapio-cli new ../my-contract --name my-contract
cd ../my-contract
cargo test --locked
```

Continue with the generated README for artifact inspection, explicit local
WASM evaluation/signing, response import and finalization. Contract logic stays
separate from the runner and tests. The project includes its dependency pins,
lockfile, toolchain and evaluator; no manually cloned dependency repositories
or WASM compiler are needed. Funding is synthetic and keys are public demo
material. Nothing is broadcast.

The [walkthrough check](docs/QUICKSTART.md#check-the-published-walkthrough)
executes the generated README outside the repository workspace and is included
in native CI.

Native CTV compilation is a **research target**. A generated address does not
establish that the target chain enforces CTV. Signer emulation has separate trust
and availability assumptions. See the [enforcement model](docs/MODERNIZATION.md#enforcement-and-release-boundaries)
before using either with funds.

For WASM compiler modules and repository checks, follow the
[development guide](docs/DEVELOPMENT.md). The
[Designing Bitcoin Contracts with Sapio](docs/learn-sapio/src/SUMMARY.md) book
starts with the maintained tutorial and retains broader historical material.

Start with [transaction plans and typed actions](docs/TRANSACTION_PLANS.md),
then [spend planning](docs/SPEND_PLANNING.md),
[selected spend completion](docs/SPEND_COMPLETION.md), and the
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
| [sapio-psbt](sapio-psbt/) | PSBT validation, scoped signing and witness finalization |
| [sapio_macros](sapio_macros/) | Rust contract authoring macros |
| [cli](cli/) | Contract, PSBT and emulator commands |
| [plugins](plugins/) | WASM client ABI and host runtime |
| [ctv_emulators](ctv_emulators/) | Covenant emulation, WASM evaluation and selected-spend completion |
| [sapio-contrib](sapio-contrib/) | Contract library and research examples |
| [plugin-example](plugin-example/) | Separately built WASM example workspace |
| [integration_tests](integration_tests/) | Compilation, signing and finalization checks |

Read Jeremy Rubin's [A Calculus of Covenants](https://rubin.io/bitcoin/2022/04/12/calc-cov/)
for the conceptual foundation. Development should preserve the connection between
intended transitions, their verifier, their prover, and the assumptions under
which they agree.

Sapio is licensed under [MPL-2.0](LICENSE). Existing ownership and contribution
terms have not been changed by this modernization.
