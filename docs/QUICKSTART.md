# Build and spend your first contract

This is the maintained entry point for Sapio's developer preview. You will
generate a small Rust project, compile a payment contract, inspect its artifact,
and complete a WASM-authorized spend through separate CLI commands. The example
uses synthetic funding and public demonstration keys; it performs no wallet
activity or broadcasting.

## Install and create a project

Install Rust with rustup, Git and a native C compiler. From a fresh Sapio checkout:

```sh
git clone https://github.com/sapio-lang/sapio.git
cd sapio
cargo build --locked -p sapio-cli
export PATH="$PWD/target/debug:$PATH"
sapio-cli new ../my-contract --name my-contract
cd ../my-contract
cargo test --locked
```

Rustup selects the checked-in toolchain. The generated project has a complete
pinned manifest, lockfile, toolchain and evaluator binary. Cargo obtains its
dependencies from their pinned repositories; do not clone Bitcoin, Miniscript
or Schemars into sibling directories. The project can live outside the Sapio
workspace. Existing destination directories are rejected.

The command above assumes Cargo's default target directory. If you set
`CARGO_TARGET_DIR`, put that directory's `debug` subdirectory on `PATH` instead.
Building compiler plugins as WASM is a separate workflow requiring LLVM Clang;
the starter uses its included evaluator bytes and needs no WASM compiler.

## Follow the generated walkthrough

Continue with the generated `README.md`, also available as the
[starter walkthrough](../cli/templates/starter/README.md). It provides every file
and command needed to:

1. Compile a typed payment action using `TemplatePlan`.
2. Inspect the raw artifact, payment/change allocations and program policy.
3. Prepare one exact branch and inspect the missing signature.
4. Explicitly run the local WASM evaluator and oracle signer.
5. Merge its verified response, resume from files and finalize the spend.
6. Observe a proposed underpayment being rejected by the evaluator.

The contract is in `src/contract.rs`; synthetic funding and file export are in
`src/main.rs`, and behavioral checks are in `src/tests.rs`. The contract requires
one fixed Program authorization and the tutorial selects its key path.
Its result is an ordinary Taproot spend; Bitcoin
does not execute the WASM program and trusts the oracle signature.

The tutorial's authorization depends on that oracle enforcing the committed
recipient and minimum honestly. Its fee reservation is local preparation policy.
Successful local finalization does not establish chain existence, unspentness,
confirmation, relay policy or signer availability. The generated keys are public
demonstration material and must never secure funds.

## Check the published walkthrough

Repository maintainers can check the shipped instructions from the Sapio root:

```sh
cargo build --locked -p sapio-cli
python3 contrib/check_quickstart.py target/debug/sapio-cli
```

The [check](../contrib/check_quickstart.py) generates a project in a temporary
directory outside Sapio's workspace and executes the three shell blocks from
its README. It checks successful completion, rejection of the underpayment,
refusal to overwrite an existing project and preservation of the lockfile.
The [native CI workflow](../.github/workflows/rust.yml) runs it on Linux and macOS;
the results for a given commit establish whether those checks passed.
Use your actual CLI binary path if you set `CARGO_TARGET_DIR`.

## Continue learning

Read [transaction plans and typed actions](TRANSACTION_PLANS.md),
[spend planning](SPEND_PLANNING.md) and
[selected spend completion](SPEND_COMPLETION.md) for the API boundaries used by
the starter. [Program emulation](PROGRAM_EMULATION.md) explains the evaluator and
protocol; [development](DEVELOPMENT.md) covers the full native/WASM checks.

The older book contains useful conceptual material and historical sketches.
Its maintained first lesson includes the starter's actual contract source;
other examples are not all standalone, compilable tutorials. The
[release-readiness record](RELAUNCH.md) distinguishes this developer path from
the remaining requirements for a supported deployment release.
